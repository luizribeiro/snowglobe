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
mod results;

// Re-export public types
pub use config::{DeploymentMode, PushConfiguration};
pub use context::PushContext;
pub use results::{BlockedHost, PushPlan, PushResults};

use crate::error::Result;
use crate::nix::{Flake, HostMetadata, StorePath};
use crate::observer::PushObserver;
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
    phase: &str,
    phase_hosts: &[HostMetadata],
    flake: &Flake,
    push_ctx: &PushContext,
    observer: &dyn PushObserver,
) -> Result<Vec<(String, crate::backends::deploy::DeployResult)>> {
    use tracing::info_span;

    // Sort hosts lexicographically for consistent rendering order
    let mut sorted_hosts = phase_hosts.to_vec();
    sorted_hosts.sort_by(|a, b| a.name.cmp(&b.name));

    // Create phase span
    let phase_span = info_span!("phase", phase = phase, hosts = sorted_hosts.len());
    // Provide phase span to observer so step spans can use it as parent
    observer.set_phase_span(phase_span.clone());
    // Pre-create step spans in the desired order for consistent rendering
    observer.prepare_step_spans();
    let _entered_phase = phase_span.enter();

    // Enter all step spans in REVERSE order to display in correct order
    // Note: Must enter in reverse because tracing-indicatif uses insert_after()
    // which causes each new child to push previous children down
    let build_span = observer.build_span();
    let copy_span = observer.copy_span();
    let activate_span = observer.activate_span();
    let reboot_span = observer.reboot_span();
    let _entered_reboot = reboot_span.as_ref().map(|s| s.enter());
    let _entered_activate = activate_span.as_ref().map(|s| s.enter());
    let _entered_copy = copy_span.as_ref().map(|s| s.enter());
    let _entered_build = build_span.as_ref().map(|s| s.enter());

    // Log phase start
    if phase == "default" || phase == "_all" {
        tracing::info!(hosts = sorted_hosts.len(), "Starting deployment");
    } else {
        tracing::info!(phase = phase, hosts = sorted_hosts.len(), "Starting phase");
    }

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
        if phase == "default" || phase == "_all" {
            tracing::error!("✗ Deployment failed");
        } else {
            tracing::error!(phase = phase, "✗ Phase failed");
        }
        return Err(err);
    }

    // Log phase success
    if phase == "default" || phase == "_all" {
        tracing::info!(
            succeeded = phase_results.len(),
            "✓ Deployment completed successfully"
        );
    } else {
        tracing::info!(
            phase = phase,
            succeeded = phase_results.len(),
            "✓ Phase completed successfully"
        );
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

/// Strategy for handling a host deployment
enum ActivationStrategy {
    /// Deploy with the given activation mode and optional reboot check
    Deploy(
        crate::backends::deploy::ActivationMode,
        Option<crate::backends::reboot::RebootCheck>,
    ),
    /// Block deployment with the given reason
    Block(String),
}

/// Determine the activation strategy for a single host
fn determine_activation_strategy(
    metadata: &HostMetadata,
    target: &crate::backends::deploy::DeployTarget,
    store_path: &StorePath,
    reboot_backend: &dyn crate::backends::reboot::RebootBackend,
    mode: DeploymentMode,
    force: bool,
) -> Result<ActivationStrategy> {
    match mode {
        DeploymentMode::DryRun => Ok(ActivationStrategy::Deploy(
            crate::backends::deploy::ActivationMode::DryActivate,
            None,
        )),
        DeploymentMode::Test => Ok(ActivationStrategy::Deploy(
            crate::backends::deploy::ActivationMode::Test,
            None,
        )),
        DeploymentMode::Boot => Ok(ActivationStrategy::Deploy(
            crate::backends::deploy::ActivationMode::Boot,
            None,
        )),
        DeploymentMode::Switch => {
            handle_switch_mode(metadata, target, store_path, reboot_backend, force)
        }
        DeploymentMode::Auto => handle_auto_mode(metadata, target, store_path, reboot_backend),
    }
}

/// Handle switch mode: force immediate activation with safety checks
fn handle_switch_mode(
    metadata: &HostMetadata,
    target: &crate::backends::deploy::DeployTarget,
    store_path: &StorePath,
    reboot_backend: &dyn crate::backends::reboot::RebootBackend,
    force: bool,
) -> Result<ActivationStrategy> {
    use crate::nix::RebootPolicy;

    // Check reboot policy - 'never' and 'always' policies override switch mode
    match metadata.reboot_policy {
        RebootPolicy::Never => {
            // Never policy: use boot mode without reboot
            return Ok(ActivationStrategy::Deploy(
                crate::backends::deploy::ActivationMode::Boot,
                Some(crate::backends::reboot::RebootCheck::not_required()),
            ));
        }
        RebootPolicy::Always => {
            // Always policy: use boot mode and will reboot
            return Ok(ActivationStrategy::Deploy(
                crate::backends::deploy::ActivationMode::Boot,
                Some(crate::backends::reboot::RebootCheck::required(
                    crate::backends::reboot::RebootReason::Policy,
                )),
            ));
        }
        RebootPolicy::Auto => {
            // Continue with normal switch mode logic
        }
    }

    let check = reboot_backend.check_reboot_required(target, store_path)?;

    if check.required && !force {
        // Reboot required but --force not set - block this host
        let reason = check
            .reason
            .as_ref()
            .map(|r| r.to_string())
            .unwrap_or_else(|| "unknown".to_string());
        Ok(ActivationStrategy::Block(reason))
    } else {
        Ok(ActivationStrategy::Deploy(
            crate::backends::deploy::ActivationMode::Switch,
            Some(check),
        ))
    }
}

/// Handle auto mode: detect reboot requirement and choose activation mode
fn handle_auto_mode(
    metadata: &HostMetadata,
    target: &crate::backends::deploy::DeployTarget,
    store_path: &StorePath,
    reboot_backend: &dyn crate::backends::reboot::RebootBackend,
) -> Result<ActivationStrategy> {
    use crate::nix::RebootPolicy;

    // Check reboot policy
    match metadata.reboot_policy {
        RebootPolicy::Never => {
            // Never policy: always use boot mode, never reboot
            return Ok(ActivationStrategy::Deploy(
                crate::backends::deploy::ActivationMode::Boot,
                Some(crate::backends::reboot::RebootCheck::not_required()),
            ));
        }
        RebootPolicy::Always => {
            // Always policy: always use boot mode and always reboot
            return Ok(ActivationStrategy::Deploy(
                crate::backends::deploy::ActivationMode::Boot,
                Some(crate::backends::reboot::RebootCheck::required(
                    crate::backends::reboot::RebootReason::Policy,
                )),
            ));
        }
        RebootPolicy::Auto => {
            // Continue with automatic detection
        }
    }

    let check = reboot_backend.check_reboot_required(target, store_path)?;

    let activation = if check.required {
        crate::backends::deploy::ActivationMode::Boot
    } else {
        crate::backends::deploy::ActivationMode::Switch
    };

    Ok(ActivationStrategy::Deploy(activation, Some(check)))
}

/// Create deployment plans with reboot detection
///
/// This function determines the appropriate activation mode for each host based on:
/// - User-requested deployment mode
/// - Reboot detection results
/// - Force flag
///
/// Returns (plans, blocked_hosts) where blocked_hosts contains hosts that were
/// blocked by safety checks (e.g., --switch without --force when reboot required)
pub fn create_deployment_plans(
    hosts: Vec<(
        HostMetadata,
        crate::backends::deploy::DeployTarget,
        StorePath,
    )>,
    reboot_backend: &dyn crate::backends::reboot::RebootBackend,
    mode: DeploymentMode,
    force: bool,
) -> Result<(Vec<PushPlan>, Vec<BlockedHost>)> {
    let mut plans = Vec::new();
    let mut blocked = Vec::new();

    for (metadata, target, store_path) in hosts {
        match determine_activation_strategy(
            &metadata,
            &target,
            &store_path,
            reboot_backend,
            mode,
            force,
        )? {
            ActivationStrategy::Deploy(activation_mode, reboot_check) => {
                plans.push(PushPlan {
                    metadata,
                    target,
                    store_path,
                    activation_mode,
                    reboot_check,
                });
            }
            ActivationStrategy::Block(reason) => {
                blocked.push(BlockedHost {
                    hostname: metadata.name.clone(),
                    reason,
                });
            }
        }
    }

    Ok((plans, blocked))
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
    use tracing::info_span;

    let hostname = &host_metadata.name;

    // Deref Arc to get the actual references
    let flake_ref = flake.as_ref();
    let push_ctx_ref = push_ctx.as_ref();
    let observer_ref = observer.as_ref();

    // Step 1: Build
    observer_ref.on_build_start(hostname)?;
    let build_output = {
        // Create host span with build span as explicit parent
        let host_span = if let Some(parent) = observer_ref.build_span() {
            info_span!(parent: &parent, "host", host = %hostname)
        } else {
            info_span!("host", host = %hostname)
        };
        let _entered = host_span.enter();

        tracing::info!("Building configuration");

        match push_ctx_ref
            .nix_backend
            .build_host(flake_ref, hostname, push_ctx_ref.config.quiet)
        {
            Ok(store_path) => {
                tracing::info!("✓ Build completed");

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
                tracing::error!("✗ Build failed");
                observer_ref.on_build_complete(hostname, false)?;
                return Err(e);
            }
        }
    };
    observer_ref.on_build_complete(hostname, true)?;

    // Step 2: Copy
    observer_ref.on_copy_start(hostname)?;
    let copy_output = {
        // Create host span with copy span as explicit parent
        let host_span = if let Some(parent) = observer_ref.copy_span() {
            info_span!(parent: &parent, "host", host = %hostname)
        } else {
            info_span!("host", host = %hostname)
        };
        let _entered = host_span.enter();

        tracing::info!("Copying closure");

        match push_ctx_ref
            .deploy_backend
            .copy_closure(&build_output.target, &build_output.store_path)
        {
            Ok(()) => {
                tracing::info!("✓ Copy completed");

                CopyOutput {
                    metadata: build_output.metadata,
                    target: build_output.target,
                    store_path: build_output.store_path,
                }
            }
            Err(e) => {
                tracing::error!("✗ Copy failed");
                observer_ref.on_copy_complete(hostname, false)?;
                return Err(e);
            }
        }
    };
    observer_ref.on_copy_complete(hostname, true)?;

    // Step 3: Planning
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

    // Step 4: Activate
    observer_ref.on_deploy_start(hostname, plan.activation_mode)?;
    let activate_result = {
        // Create host span with activate span as explicit parent
        let host_span = if let Some(parent) = observer_ref.activate_span() {
            info_span!(parent: &parent, "host", host = %hostname)
        } else {
            info_span!("host", host = %hostname)
        };
        let _entered = host_span.enter();

        tracing::info!("Activating configuration");

        let result = push_ctx_ref
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

        if result.success {
            tracing::info!("✓ Activation completed");
        } else {
            tracing::error!(message = %result.message, "✗ Activation failed");
        }

        result
    };
    observer_ref.on_deploy_complete(hostname, &activate_result)?;

    if !activate_result.success {
        return Err(crate::error::SnowError::Deploy(
            crate::error::DeployError::Failed {
                host: hostname.clone(),
                source: std::io::Error::other(activate_result.message.clone()),
            },
        ));
    }

    // Step 5: Reboot (if needed)
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

        // Create host span with reboot span as explicit parent
        let host_span = if let Some(parent) = observer_ref.reboot_span() {
            info_span!(parent: &parent, "host", host = %hostname)
        } else {
            info_span!("host", host = %hostname)
        };
        let _entered = host_span.enter();

        tracing::info!("Rebooting host");

        match push_ctx_ref.reboot_backend.reboot_and_wait(&plan.target) {
            Ok(()) => {
                tracing::info!("✓ Back online");
                observer_ref.on_reboot_complete(hostname, true)?;
                activate_result
            }
            Err(e) => {
                tracing::error!("✗ Reboot failed");
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
mod tests {
    use super::*;
    use crate::backends::deploy::{ActivationMode, DeployResult, DeployTarget};
    use crate::backends::mock::{MockDeployBackend, MockNixBackend};
    use crate::nix::{Flake, HostMetadata, StorePath};

    /// Helper to create just HostMetadata for push tests
    fn make_host_metadata(
        name: &str,
        phase: Option<&str>,
        reboot_policy: crate::nix::RebootPolicy,
    ) -> HostMetadata {
        HostMetadata {
            name: name.to_string(),
            phase: phase.map(|s| s.to_string()),
            tags: vec![],
            fqdn: format!("{}.example.com", name),
            reboot_policy,
        }
    }

    /// Helper to create a tuple for create_deployment_plans tests
    fn make_host(
        name: &str,
        phase: Option<&str>,
        reboot_policy: crate::nix::RebootPolicy,
    ) -> (HostMetadata, DeployTarget, StorePath) {
        let metadata = make_host_metadata(name, phase, reboot_policy);

        let target = DeployTarget {
            user: Some("root".to_string()),
            hostname: format!("{}.example.com", name),
            port: None,
        };

        let store_path = StorePath::new(format!("/nix/store/fake-{}", name));

        (metadata, target, store_path)
    }

    /// Helper function to activate hosts (simplified for tests)
    fn activate_plans_for_test(
        deploy_backend: &dyn crate::backends::deploy::DeployBackend,
        plans: Vec<PushPlan>,
        _concurrency: usize,
        observer: &dyn crate::observer::PushObserver,
    ) -> Result<PushResults> {
        use crate::backends::deploy::DeployResult;

        // Get the phase from the first plan (all plans in tests have the same phase)
        let phase = plans
            .first()
            .and_then(|p| p.metadata.phase.as_ref())
            .cloned()
            .unwrap_or_else(|| "default".to_string());

        // Collect hostnames for phase lifecycle notification
        let hostnames: Vec<String> = plans.iter().map(|p| p.metadata.name.clone()).collect();

        // Notify observer of phase start
        observer.on_phase_start(&phase, &hostnames)?;

        // Activate each plan
        let mut push_results = Vec::new();
        let mut all_succeeded = true;

        for plan in plans {
            let hostname = plan.metadata.name.clone();

            // Notify deploy start
            observer.on_deploy_start(&hostname, plan.activation_mode)?;

            // Execute activation
            let result = deploy_backend
                .deploy(&plan.target, &plan.store_path, plan.activation_mode)
                .unwrap_or_else(|e| DeployResult::failure(e.to_string()));

            if !result.success {
                all_succeeded = false;
            }

            // Notify deploy complete
            observer.on_deploy_complete(&hostname, &result)?;

            push_results.push((hostname, result));
        }

        // Notify observer of phase completion
        observer.on_phase_complete(&phase, all_succeeded)?;

        Ok(PushResults {
            results: push_results,
        })
    }

    #[test]
    fn test_push_results_counts() {
        let results = PushResults {
            results: vec![
                ("host1".to_string(), DeployResult::success("ok")),
                ("host2".to_string(), DeployResult::failure("failed")),
                ("host3".to_string(), DeployResult::success("ok")),
            ],
        };

        assert_eq!(results.success_count(), 2);
        assert_eq!(results.failure_count(), 1);
        assert!(!results.all_succeeded());
    }

    #[test]
    fn test_push_results_all_succeeded() {
        let results = PushResults {
            results: vec![
                ("host1".to_string(), DeployResult::success("ok")),
                ("host2".to_string(), DeployResult::success("ok")),
            ],
        };

        assert_eq!(results.success_count(), 2);
        assert_eq!(results.failure_count(), 0);
        assert!(results.all_succeeded());
    }

    // Tests for create_deployment_plans
    use crate::backends::reboot::{RebootDetectionConfig, SystemRebootBackend};

    #[test]
    fn test_create_deployment_plans_auto_mode_no_reboot() {
        // Create a detector that will never detect reboot needed
        // This simulates the edge case where:
        // - No kernel/initrd/init changes
        // - No critical services to restart (empty list)
        // - /run/nixos/dry-activation-restart-list won't exist on remote
        // This configuration was causing "No such file or directory" errors
        // before we added the file existence check in check_critical_services()
        let config = RebootDetectionConfig {
            critical_services: vec![],
            check_kernel: false,
            check_initrd: false,
            check_init: false,
        };
        let detector = SystemRebootBackend::new(config);

        let hosts = vec![make_host("test", None, crate::nix::RebootPolicy::Auto)];

        let (plans, blocked) =
            create_deployment_plans(hosts, &detector, DeploymentMode::Auto, false).unwrap();

        assert_eq!(plans.len(), 1);
        assert_eq!(blocked.len(), 0);
        // Should use Switch mode since no reboot is required
        assert_eq!(plans[0].activation_mode, ActivationMode::Switch);
        // Reboot check should indicate no reboot needed
        assert!(plans[0].reboot_check.is_some());
        assert!(!plans[0].reboot_check.as_ref().unwrap().required);
    }

    #[test]
    fn test_create_deployment_plans_dry_run() {
        let detector = SystemRebootBackend::default();
        let hosts = vec![make_host("test", None, crate::nix::RebootPolicy::Auto)];

        let (plans, blocked) =
            create_deployment_plans(hosts, &detector, DeploymentMode::DryRun, false).unwrap();

        assert_eq!(plans.len(), 1);
        assert_eq!(blocked.len(), 0);
        assert_eq!(plans[0].activation_mode, ActivationMode::DryActivate);
        assert!(plans[0].reboot_check.is_none());
    }

    #[test]
    fn test_create_deployment_plans_boot_mode() {
        let detector = SystemRebootBackend::default();
        let hosts = vec![make_host("test", None, crate::nix::RebootPolicy::Auto)];

        let (plans, blocked) =
            create_deployment_plans(hosts, &detector, DeploymentMode::Boot, false).unwrap();

        assert_eq!(plans.len(), 1);
        assert_eq!(blocked.len(), 0);
        assert_eq!(plans[0].activation_mode, ActivationMode::Boot);
        assert!(plans[0].reboot_check.is_none());
    }

    #[test]
    fn test_create_deployment_plans_test_mode() {
        let detector = SystemRebootBackend::default();
        let hosts = vec![make_host("test", None, crate::nix::RebootPolicy::Auto)];

        let (plans, blocked) =
            create_deployment_plans(hosts, &detector, DeploymentMode::Test, false).unwrap();

        assert_eq!(plans.len(), 1);
        assert_eq!(blocked.len(), 0);
        assert_eq!(plans[0].activation_mode, ActivationMode::Test);
        assert!(plans[0].reboot_check.is_none());
    }

    #[test]
    fn test_create_deployment_plans_switch_with_force() {
        // In reality, reboot would be detected, but with force flag it should still use Switch
        let detector = SystemRebootBackend::default();
        let hosts = vec![make_host("test", None, crate::nix::RebootPolicy::Auto)];

        let (plans, blocked) =
            create_deployment_plans(hosts, &detector, DeploymentMode::Switch, true).unwrap();

        assert_eq!(plans.len(), 1);
        assert_eq!(blocked.len(), 0);
        // Force flag causes switch mode even if reboot detected
        assert_eq!(plans[0].activation_mode, ActivationMode::Switch);
    }

    #[test]
    fn test_create_deployment_plans_multiple_hosts_mixed_results() {
        // Create a detector that will never detect reboot needed
        let config = RebootDetectionConfig {
            critical_services: vec![],
            check_kernel: false,
            check_initrd: false,
            check_init: false,
        };
        let detector = SystemRebootBackend::new(config);

        let hosts = vec![
            make_host("host1", None, crate::nix::RebootPolicy::Auto),
            make_host("host2", None, crate::nix::RebootPolicy::Auto),
            make_host("host3", None, crate::nix::RebootPolicy::Auto),
        ];

        let (plans, blocked) =
            create_deployment_plans(hosts, &detector, DeploymentMode::Auto, false).unwrap();

        assert_eq!(plans.len(), 3);
        assert_eq!(blocked.len(), 0);
        // All should use switch mode since no reboot detected
        for plan in plans {
            assert_eq!(plan.activation_mode, ActivationMode::Switch);
        }
    }

    #[test]
    fn test_reboot_policy_never_uses_boot_mode() {
        let config = RebootDetectionConfig {
            critical_services: vec![],
            check_kernel: false,
            check_initrd: false,
            check_init: false,
        };
        let detector = SystemRebootBackend::new(config);

        let hosts = vec![make_host("host1", None, crate::nix::RebootPolicy::Never)];

        let (plans, blocked) =
            create_deployment_plans(hosts, &detector, DeploymentMode::Auto, false).unwrap();

        assert_eq!(plans.len(), 1);
        assert_eq!(blocked.len(), 0);
        // Should use boot mode due to 'never' policy
        assert_eq!(plans[0].activation_mode, ActivationMode::Boot);
        // Should indicate no reboot required
        assert!(plans[0].reboot_check.is_some());
        assert!(!plans[0].reboot_check.as_ref().unwrap().required);
    }

    #[test]
    fn test_reboot_policy_always_uses_boot_mode_with_reboot() {
        let config = RebootDetectionConfig {
            critical_services: vec![],
            check_kernel: false,
            check_initrd: false,
            check_init: false,
        };
        let detector = SystemRebootBackend::new(config);

        let hosts = vec![make_host("host1", None, crate::nix::RebootPolicy::Always)];

        let (plans, blocked) =
            create_deployment_plans(hosts, &detector, DeploymentMode::Auto, false).unwrap();

        assert_eq!(plans.len(), 1);
        assert_eq!(blocked.len(), 0);
        // Should use boot mode due to 'always' policy
        assert_eq!(plans[0].activation_mode, ActivationMode::Boot);
        // Should indicate reboot required
        assert!(plans[0].reboot_check.is_some());
        assert!(plans[0].reboot_check.as_ref().unwrap().required);
    }

    #[test]
    fn test_reboot_policy_auto_uses_detection() {
        let config = RebootDetectionConfig {
            critical_services: vec![],
            check_kernel: false,
            check_initrd: false,
            check_init: false,
        };
        let detector = SystemRebootBackend::new(config);

        let hosts = vec![make_host("host1", None, crate::nix::RebootPolicy::Auto)];

        let (plans, blocked) =
            create_deployment_plans(hosts, &detector, DeploymentMode::Auto, false).unwrap();

        assert_eq!(plans.len(), 1);
        assert_eq!(blocked.len(), 0);
        // Should use switch mode (no reboot detected)
        assert_eq!(plans[0].activation_mode, ActivationMode::Switch);
        assert!(plans[0].reboot_check.is_some());
        assert!(!plans[0].reboot_check.as_ref().unwrap().required);
    }

    #[test]
    fn test_reboot_policy_never_overrides_switch_mode() {
        let config = RebootDetectionConfig {
            critical_services: vec![],
            check_kernel: false,
            check_initrd: false,
            check_init: false,
        };
        let detector = SystemRebootBackend::new(config);

        let hosts = vec![make_host("host1", None, crate::nix::RebootPolicy::Never)];

        let (plans, blocked) =
            create_deployment_plans(hosts, &detector, DeploymentMode::Switch, false).unwrap();

        assert_eq!(plans.len(), 1);
        assert_eq!(blocked.len(), 0);
        // Should use boot mode despite switch flag (policy overrides)
        assert_eq!(plans[0].activation_mode, ActivationMode::Boot);
        assert!(!plans[0].reboot_check.as_ref().unwrap().required);
    }

    #[test]
    fn test_reboot_policy_always_overrides_switch_mode() {
        let config = RebootDetectionConfig {
            critical_services: vec![],
            check_kernel: false,
            check_initrd: false,
            check_init: false,
        };
        let detector = SystemRebootBackend::new(config);

        let hosts = vec![make_host("host1", None, crate::nix::RebootPolicy::Always)];

        let (plans, blocked) =
            create_deployment_plans(hosts, &detector, DeploymentMode::Switch, false).unwrap();

        assert_eq!(plans.len(), 1);
        assert_eq!(blocked.len(), 0);
        // Should use boot mode despite switch flag (policy overrides)
        assert_eq!(plans[0].activation_mode, ActivationMode::Boot);
        assert!(plans[0].reboot_check.as_ref().unwrap().required);
    }

    // Tests for the new push() function with observer support
    use crate::observer::{NoopObserver, PushObserver};
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct TestObserver {
        events: Arc<Mutex<Vec<String>>>,
    }

    impl TestObserver {
        fn new() -> Self {
            Self {
                events: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn get_events(&self) -> Vec<String> {
            self.events.lock().unwrap().clone()
        }
    }

    impl PushObserver for TestObserver {
        fn on_build_step_start(&self, phase: &str, hosts: &[String]) -> crate::error::Result<()> {
            self.events
                .lock()
                .unwrap()
                .push(format!("build_step_start:{}:{}", phase, hosts.len()));
            Ok(())
        }

        fn on_build_start(&self, hostname: &str) -> crate::error::Result<()> {
            self.events
                .lock()
                .unwrap()
                .push(format!("build_start:{}", hostname));
            Ok(())
        }

        fn on_build_complete(&self, hostname: &str, success: bool) -> crate::error::Result<()> {
            self.events.lock().unwrap().push(format!(
                "build_complete:{}:{}",
                hostname,
                if success { "ok" } else { "fail" }
            ));
            Ok(())
        }

        fn on_build_step_complete(&self, phase: &str, success: bool) -> crate::error::Result<()> {
            self.events.lock().unwrap().push(format!(
                "build_step_complete:{}:{}",
                phase,
                if success { "ok" } else { "fail" }
            ));
            Ok(())
        }

        fn on_copy_step_start(&self, phase: &str, hosts: &[String]) -> crate::error::Result<()> {
            self.events
                .lock()
                .unwrap()
                .push(format!("copy_step_start:{}:{}", phase, hosts.len()));
            Ok(())
        }

        fn on_copy_start(&self, hostname: &str) -> crate::error::Result<()> {
            self.events
                .lock()
                .unwrap()
                .push(format!("copy_start:{}", hostname));
            Ok(())
        }

        fn on_copy_complete(&self, hostname: &str, success: bool) -> crate::error::Result<()> {
            self.events.lock().unwrap().push(format!(
                "copy_complete:{}:{}",
                hostname,
                if success { "ok" } else { "fail" }
            ));
            Ok(())
        }

        fn on_copy_step_complete(&self, phase: &str, success: bool) -> crate::error::Result<()> {
            self.events.lock().unwrap().push(format!(
                "copy_step_complete:{}:{}",
                phase,
                if success { "ok" } else { "fail" }
            ));
            Ok(())
        }

        fn on_phase_start(&self, phase: &str, hosts: &[String]) -> crate::error::Result<()> {
            self.events
                .lock()
                .unwrap()
                .push(format!("phase_start:{}:{}", phase, hosts.len()));
            Ok(())
        }

        fn on_deploy_step_start(&self, phase: &str, hosts: &[String]) -> crate::error::Result<()> {
            self.events.lock().unwrap().push(format!(
                "deploy_step_start:{}:{}",
                phase,
                hosts.len()
            ));
            Ok(())
        }

        fn on_deploy_start(
            &self,
            hostname: &str,
            _mode: ActivationMode,
        ) -> crate::error::Result<()> {
            self.events
                .lock()
                .unwrap()
                .push(format!("deploy_start:{}", hostname));
            Ok(())
        }

        fn on_deploy_complete(
            &self,
            hostname: &str,
            result: &DeployResult,
        ) -> crate::error::Result<()> {
            self.events.lock().unwrap().push(format!(
                "deploy_complete:{}:{}",
                hostname,
                if result.success { "ok" } else { "failed" }
            ));
            Ok(())
        }

        fn on_deploy_step_complete(&self, phase: &str, success: bool) -> crate::error::Result<()> {
            self.events.lock().unwrap().push(format!(
                "deploy_step_complete:{}:{}",
                phase,
                if success { "ok" } else { "fail" }
            ));
            Ok(())
        }

        fn on_phase_complete(&self, phase: &str, success: bool) -> crate::error::Result<()> {
            self.events.lock().unwrap().push(format!(
                "phase_complete:{}:{}",
                phase,
                if success { "ok" } else { "failed" }
            ));
            Ok(())
        }

        fn on_reboot_start(&self, hostname: &str) -> crate::error::Result<()> {
            self.events
                .lock()
                .unwrap()
                .push(format!("reboot_start:{}", hostname));
            Ok(())
        }

        fn on_reboot_complete(&self, hostname: &str, success: bool) -> crate::error::Result<()> {
            self.events.lock().unwrap().push(format!(
                "reboot_complete:{}:{}",
                hostname,
                if success { "ok" } else { "fail" }
            ));
            Ok(())
        }

        fn on_reboot_step_start(&self, phase: &str, hosts: &[String]) -> crate::error::Result<()> {
            self.events.lock().unwrap().push(format!(
                "reboot_step_start:{}:{}",
                phase,
                hosts.len()
            ));
            Ok(())
        }

        fn on_reboot_step_complete(&self, phase: &str, success: bool) -> crate::error::Result<()> {
            self.events.lock().unwrap().push(format!(
                "reboot_step_complete:{}:{}",
                phase,
                if success { "ok" } else { "fail" }
            ));
            Ok(())
        }
    }

    #[test]
    fn test_push_with_single_phase() {
        let deployer = MockDeployBackend::new();
        let observer = TestObserver::new();

        let plans = vec![
            PushPlan {
                metadata: HostMetadata {
                    name: "host1".to_string(),
                    phase: Some("p1".to_string()),
                    tags: vec![],
                    fqdn: "host1.example.com".to_string(),
                    reboot_policy: crate::nix::RebootPolicy::Auto,
                },
                target: DeployTarget {
                    user: Some("root".to_string()),
                    hostname: "host1.example.com".to_string(),
                    port: None,
                },
                store_path: StorePath::new("/nix/store/fake-host1".to_string()),
                activation_mode: ActivationMode::Switch,
                reboot_check: None,
            },
            PushPlan {
                metadata: HostMetadata {
                    name: "host2".to_string(),
                    phase: Some("p1".to_string()),
                    tags: vec![],
                    fqdn: "host2.example.com".to_string(),
                    reboot_policy: crate::nix::RebootPolicy::Auto,
                },
                target: DeployTarget {
                    user: Some("root".to_string()),
                    hostname: "host2.example.com".to_string(),
                    port: None,
                },
                store_path: StorePath::new("/nix/store/fake-host2".to_string()),
                activation_mode: ActivationMode::Switch,
                reboot_check: None,
            },
        ];

        let results = activate_plans_for_test(&deployer, plans, 2, &observer).unwrap();

        assert_eq!(results.success_count(), 2);
        assert_eq!(results.failure_count(), 0);

        // Check observer events
        let events = observer.get_events();
        assert!(events.contains(&"phase_start:p1:2".to_string()));
        assert!(events.contains(&"deploy_start:host1".to_string()));
        assert!(events.contains(&"deploy_start:host2".to_string()));
        assert!(events.contains(&"deploy_complete:host1:ok".to_string()));
        assert!(events.contains(&"deploy_complete:host2:ok".to_string()));
        assert!(events.contains(&"phase_complete:p1:ok".to_string()));
    }

    #[test]
    fn test_push_with_multiple_phases() {
        // Test that phases execute in lexicographic order (p1 before p2)
        let flake = Flake::test_context();
        let nix_backend = MockNixBackend::new();
        let deployer = MockDeployBackend::new();
        let reboot_config = crate::backends::reboot::RebootDetectionConfig {
            critical_services: vec![],
            check_kernel: false,
            check_initrd: false,
            check_init: false,
        };
        let reboot_detector = crate::backends::reboot::SystemRebootBackend::new(reboot_config);
        let observer = TestObserver::new();

        // Note: hosts are intentionally out of order to test sorting
        let hosts = vec![
            HostMetadata {
                name: "host3".to_string(),
                fqdn: "host3.example.com".to_string(),
                phase: Some("p2".to_string()),
                tags: vec![],
                reboot_policy: crate::nix::RebootPolicy::Auto,
            },
            HostMetadata {
                name: "host1".to_string(),
                fqdn: "host1.example.com".to_string(),
                phase: Some("p1".to_string()),
                tags: vec![],
                reboot_policy: crate::nix::RebootPolicy::Auto,
            },
        ];

        let config = PushConfiguration {
            mode: DeploymentMode::Switch,
            force: false,
            concurrency: 2,
            ignore_phases: false,
            quiet: false,
            ssh_user: None,
            allow_reboot: true,
        };

        let results = {
            let push_ctx = PushContext::new(&nix_backend, &deployer, &reboot_detector, &config);
            push(&flake, &push_ctx, hosts, &observer)
        }
        .unwrap();

        assert_eq!(results.success_count(), 2);

        // Check that phases were executed in order
        let events = observer.get_events();
        let p1_start = events.iter().position(|e| e == "phase_start:p1:1").unwrap();
        let p2_start = events.iter().position(|e| e == "phase_start:p2:1").unwrap();

        // p1 should start before p2 (lexicographic order)
        assert!(p1_start < p2_start);
    }

    #[test]
    fn test_push_stops_on_phase_failure() {
        // Test that when phase p1 fails, phase p2 never starts
        let flake = Flake::test_context();
        let nix_backend = MockNixBackend::new();
        let deployer =
            MockDeployBackend::new().with_activate_failures(vec!["host1.example.com".to_string()]);
        let reboot_config = crate::backends::reboot::RebootDetectionConfig {
            critical_services: vec![],
            check_kernel: false,
            check_initrd: false,
            check_init: false,
        };
        let reboot_detector = crate::backends::reboot::SystemRebootBackend::new(reboot_config);
        let observer = TestObserver::new();

        let hosts = vec![
            HostMetadata {
                name: "host1".to_string(),
                fqdn: "host1.example.com".to_string(),
                phase: Some("p1".to_string()),
                tags: vec![],
                reboot_policy: crate::nix::RebootPolicy::Auto,
            },
            HostMetadata {
                name: "host2".to_string(),
                fqdn: "host2.example.com".to_string(),
                phase: Some("p2".to_string()),
                tags: vec![],
                reboot_policy: crate::nix::RebootPolicy::Auto,
            },
        ];

        let config = PushConfiguration {
            mode: DeploymentMode::Switch,
            force: false,
            concurrency: 2,
            ignore_phases: false,
            quiet: false,
            ssh_user: None,
            allow_reboot: true,
        };

        // When activation fails, push() returns Err
        let result = {
            let push_ctx = PushContext::new(&nix_backend, &deployer, &reboot_detector, &config);
            push(&flake, &push_ctx, hosts, &observer)
        };

        // Push should fail due to phase p1 activation failure
        assert!(
            result.is_err(),
            "Push should return error when activation fails"
        );

        let events = observer.get_events();

        // Phase p1 should have completed (with failure)
        assert!(events.contains(&"phase_complete:p1:failed".to_string()));

        // Phase p2 should never start (no build, copy, or deploy events for host2)
        assert!(
            !events.iter().any(|e| e.contains("host2")),
            "Phase p2 should not have started after p1 failure"
        );
    }

    #[test]
    fn test_push_with_noop_observer() {
        let deployer = MockDeployBackend::new();
        let observer = NoopObserver;

        let plans = vec![PushPlan {
            metadata: HostMetadata {
                name: "host1".to_string(),
                phase: Some("p1".to_string()),
                tags: vec![],
                fqdn: "host1.example.com".to_string(),
                reboot_policy: crate::nix::RebootPolicy::Auto,
            },
            target: DeployTarget {
                user: Some("root".to_string()),
                hostname: "host1.example.com".to_string(),
                port: None,
            },
            store_path: StorePath::new("/nix/store/fake-host1".to_string()),
            activation_mode: ActivationMode::Switch,
            reboot_check: None,
        }];

        let results = activate_plans_for_test(&deployer, plans, 1, &observer).unwrap();
        assert_eq!(results.success_count(), 1);
    }

    #[test]
    fn test_push_with_default_phase() {
        let deployer = MockDeployBackend::new();
        let observer = TestObserver::new();

        let plans = vec![PushPlan {
            metadata: HostMetadata {
                name: "host1".to_string(),
                phase: None, // No phase specified
                tags: vec![],
                fqdn: "host1.example.com".to_string(),
                reboot_policy: crate::nix::RebootPolicy::Auto,
            },
            target: DeployTarget {
                user: Some("root".to_string()),
                hostname: "host1.example.com".to_string(),
                port: None,
            },
            store_path: StorePath::new("/nix/store/fake-host1".to_string()),
            activation_mode: ActivationMode::Switch,
            reboot_check: None,
        }];

        let results = activate_plans_for_test(&deployer, plans, 1, &observer).unwrap();
        assert_eq!(results.success_count(), 1);

        // Should use "default" phase
        let events = observer.get_events();
        assert!(events.contains(&"phase_start:default:1".to_string()));
    }

    #[test]
    fn test_push_with_partial_failure_in_phase() {
        // Test that when one host fails in a phase, we still get results for all hosts in that phase
        let deployer =
            MockDeployBackend::new().with_failures(vec!["host2.example.com".to_string()]);
        let observer = TestObserver::new();

        let plans = vec![
            PushPlan {
                metadata: HostMetadata {
                    name: "host1".to_string(),
                    phase: Some("p1".to_string()),
                    tags: vec![],
                    fqdn: "host1.example.com".to_string(),
                    reboot_policy: crate::nix::RebootPolicy::Auto,
                },
                target: DeployTarget {
                    user: Some("root".to_string()),
                    hostname: "host1.example.com".to_string(),
                    port: None,
                },
                store_path: StorePath::new("/nix/store/fake-host1".to_string()),
                activation_mode: ActivationMode::Switch,
                reboot_check: None,
            },
            PushPlan {
                metadata: HostMetadata {
                    name: "host2".to_string(),
                    phase: Some("p1".to_string()),
                    tags: vec![],
                    fqdn: "host2.example.com".to_string(),
                    reboot_policy: crate::nix::RebootPolicy::Auto,
                },
                target: DeployTarget {
                    user: Some("root".to_string()),
                    hostname: "host2.example.com".to_string(),
                    port: None,
                },
                store_path: StorePath::new("/nix/store/fake-host2".to_string()),
                activation_mode: ActivationMode::Switch,
                reboot_check: None,
            },
        ];

        let results = activate_plans_for_test(&deployer, plans, 2, &observer).unwrap();

        // Both hosts should have results
        assert_eq!(results.results.len(), 2);
        assert_eq!(results.success_count(), 1);
        assert_eq!(results.failure_count(), 1);

        // Check that one succeeded and one failed
        let hostnames: Vec<&str> = results.results.iter().map(|(n, _)| n.as_str()).collect();
        assert!(hostnames.contains(&"host1"));
        assert!(hostnames.contains(&"host2"));

        let host1_result = results.results.iter().find(|(n, _)| n == "host1").unwrap();
        let host2_result = results.results.iter().find(|(n, _)| n == "host2").unwrap();
        assert!(host1_result.1.success);
        assert!(!host2_result.1.success);

        // Phase should be marked as failed
        let events = observer.get_events();
        assert!(events.contains(&"phase_complete:p1:failed".to_string()));
    }

    #[test]
    fn test_push_error_message_includes_source() {
        // Test that when deployment fails, the error message includes the full error chain
        let flake = Flake::test_context();
        let nix_backend = MockNixBackend::new();
        let deployer =
            MockDeployBackend::new().with_activate_failures(vec!["fail.example.com".to_string()]);

        // Use a reboot detector that won't require reboots (empty config)
        let reboot_config = crate::backends::reboot::RebootDetectionConfig {
            critical_services: vec![],
            check_kernel: false,
            check_initrd: false,
            check_init: false,
        };
        let reboot_detector = crate::backends::reboot::SystemRebootBackend::new(reboot_config);
        let observer = crate::observer::NoopObserver;

        let hosts = vec![HostMetadata {
            name: "fail".to_string(),
            fqdn: "fail.example.com".to_string(),
            phase: Some("p1".to_string()),
            tags: vec![],
            reboot_policy: crate::nix::RebootPolicy::Auto,
        }];

        let config = PushConfiguration {
            mode: DeploymentMode::Switch,
            force: false,
            concurrency: 1,
            ignore_phases: false,
            quiet: false,
            ssh_user: None,
            allow_reboot: true,
        };

        // When activation fails, push() returns Err
        let result = {
            let push_ctx = PushContext::new(&nix_backend, &deployer, &reboot_detector, &config);
            push(&flake, &push_ctx, hosts, &observer)
        };

        // Push should fail due to activation failure
        assert!(
            result.is_err(),
            "Push should return error when activation fails"
        );

        // Verify the error message includes the source error chain
        let error = result.unwrap_err();

        // Build full error message including source chain
        let mut full_error = error.to_string();
        let mut source = std::error::Error::source(&error);
        while let Some(err) = source {
            full_error.push_str(&format!("\n  Caused by: {}", err));
            source = std::error::Error::source(err);
        }

        assert!(
            full_error.contains("fail"),
            "Error message should include the failed host name. Got: {}",
            full_error
        );
        assert!(
            full_error.contains("activation failed"),
            "Error message should include 'activation failed'. Got: {}",
            full_error
        );
    }

    #[test]
    fn test_push_error_message_format_with_activation_failure() {
        // Test the specific format of error messages with activation failures
        use std::collections::HashMap;

        let mut failures = HashMap::new();
        failures.insert(
            "test.example.com".to_string(),
            crate::backends::mock::FailureSpec {
                phase: "activate".to_string(),
                error: Some("systemd service failed to start".to_string()),
            },
        );

        let fixture = crate::backends::mock::DeploymentFixture {
            name: "Test error formatting".to_string(),
            description: None,
            failures,
            hosts: vec![],
        };

        let flake = Flake::test_context();
        let nix_backend = MockNixBackend::new();
        let deployer = MockDeployBackend::from_fixture(fixture);
        let reboot_config = crate::backends::reboot::RebootDetectionConfig {
            critical_services: vec![],
            check_kernel: false,
            check_initrd: false,
            check_init: false,
        };
        let reboot_detector = crate::backends::reboot::SystemRebootBackend::new(reboot_config);
        let observer = crate::observer::NoopObserver;

        let hosts = vec![HostMetadata {
            name: "test".to_string(),
            fqdn: "test.example.com".to_string(),
            phase: Some("p1".to_string()),
            tags: vec![],
            reboot_policy: crate::nix::RebootPolicy::Auto,
        }];

        let config = PushConfiguration {
            mode: DeploymentMode::Switch,
            force: false,
            concurrency: 1,
            ignore_phases: false,
            quiet: false,
            ssh_user: None,
            allow_reboot: true,
        };

        // When activation fails, push() returns Err
        let result = {
            let push_ctx = PushContext::new(&nix_backend, &deployer, &reboot_detector, &config);
            push(&flake, &push_ctx, hosts, &observer)
        };

        // Push should fail due to activation failure
        assert!(
            result.is_err(),
            "Push should return error when activation fails"
        );

        // Verify the error message includes expected details
        let error = result.unwrap_err();

        // Build full error message including source chain
        let mut full_error = error.to_string();
        let mut source = std::error::Error::source(&error);
        while let Some(err) = source {
            full_error.push_str(&format!("\n  Caused by: {}", err));
            source = std::error::Error::source(err);
        }

        assert!(
            full_error.contains("test"),
            "Error message should include the failed host name. Got: {}",
            full_error
        );
        assert!(
            full_error.contains("systemd service failed to start"),
            "Error message should include the specific error details. Got: {}",
            full_error
        );
    }

    // ============================================================================
    // Integration Tests for push() Function
    // ============================================================================

    #[test]
    fn test_push_full_workflow_single_phase_success() {
        // Test the complete workflow: build → copy → plan → activate for a single phase
        let flake = Flake::test_context();
        let nix_backend = MockNixBackend::new();
        let deployer = MockDeployBackend::new();
        let reboot_detector = SystemRebootBackend::default();
        let observer = TestObserver::new();

        let hosts = vec![
            HostMetadata {
                name: "web1".to_string(),
                fqdn: "web1.example.com".to_string(),
                phase: Some("web".to_string()),
                tags: vec![],
                reboot_policy: crate::nix::RebootPolicy::Never, // No reboot needed
            },
            HostMetadata {
                name: "web2".to_string(),
                fqdn: "web2.example.com".to_string(),
                phase: Some("web".to_string()),
                tags: vec![],
                reboot_policy: crate::nix::RebootPolicy::Never,
            },
        ];

        let config = PushConfiguration {
            mode: DeploymentMode::Switch,
            force: false,
            concurrency: 2,
            ignore_phases: false,
            quiet: false,
            ssh_user: None,
            allow_reboot: true,
        };

        let results = {
            let push_ctx = PushContext::new(&nix_backend, &deployer, &reboot_detector, &config);
            push(&flake, &push_ctx, hosts, &observer)
        }
        .unwrap();

        // Verify both hosts succeeded
        assert_eq!(results.success_count(), 2);
        assert_eq!(results.failure_count(), 0);

        // Verify observer events fired in correct order (parallel-per-host model)
        let events = observer.get_events();

        // Build events (per-host)
        assert!(events.contains(&"build_start:web1".to_string()));
        assert!(events.contains(&"build_complete:web1:ok".to_string()));
        assert!(events.contains(&"build_start:web2".to_string()));
        assert!(events.contains(&"build_complete:web2:ok".to_string()));

        // Copy events (per-host)
        assert!(events.contains(&"copy_start:web1".to_string()));
        assert!(events.contains(&"copy_complete:web1:ok".to_string()));
        assert!(events.contains(&"copy_start:web2".to_string()));
        assert!(events.contains(&"copy_complete:web2:ok".to_string()));

        // Activation phase events
        assert!(events.contains(&"phase_start:web:2".to_string()));
        assert!(events.contains(&"deploy_start:web1".to_string()));
        assert!(events.contains(&"deploy_complete:web1:ok".to_string()));
        assert!(events.contains(&"deploy_start:web2".to_string()));
        assert!(events.contains(&"deploy_complete:web2:ok".to_string()));
        assert!(events.contains(&"phase_complete:web:ok".to_string()));
    }

    #[test]
    fn test_push_full_workflow_multi_phase_success() {
        // Test multiple phases deploying sequentially with all steps
        let flake = Flake::test_context();
        let nix_backend = MockNixBackend::new();
        let deployer = MockDeployBackend::new();
        let reboot_detector = SystemRebootBackend::default();
        let observer = TestObserver::new();

        let hosts = vec![
            HostMetadata {
                name: "db1".to_string(),
                fqdn: "db1.example.com".to_string(),
                phase: Some("database".to_string()),
                tags: vec![],
                reboot_policy: crate::nix::RebootPolicy::Never,
            },
            HostMetadata {
                name: "web1".to_string(),
                fqdn: "web1.example.com".to_string(),
                phase: Some("web".to_string()),
                tags: vec![],
                reboot_policy: crate::nix::RebootPolicy::Never,
            },
        ];

        let config = PushConfiguration {
            mode: DeploymentMode::Switch,
            force: false,
            concurrency: 2,
            ignore_phases: false,
            quiet: false,
            ssh_user: None,
            allow_reboot: true,
        };

        let results = {
            let push_ctx = PushContext::new(&nix_backend, &deployer, &reboot_detector, &config);
            push(&flake, &push_ctx, hosts, &observer)
        }
        .unwrap();

        assert_eq!(results.success_count(), 2);

        let events = observer.get_events();

        // Database phase should complete before web phase starts (parallel-per-host model)
        let db_phase_start = events
            .iter()
            .position(|e| e == "phase_start:database:1")
            .expect("database phase should start");
        let db_phase_complete = events
            .iter()
            .position(|e| e == "phase_complete:database:ok")
            .expect("database phase should complete");
        let web_phase_start = events
            .iter()
            .position(|e| e == "phase_start:web:1")
            .expect("web phase should start");

        assert!(db_phase_start < db_phase_complete);
        assert!(db_phase_complete < web_phase_start);
    }

    #[test]
    fn test_push_build_failure_stops_deployment() {
        // Build fails for one host, should stop immediately with no copy/activate
        let flake = Flake::test_context();
        let nix_backend = MockNixBackend::new().with_build_failures(vec!["fail-host".to_string()]);
        let deployer = MockDeployBackend::new();
        let reboot_detector = SystemRebootBackend::default();
        let observer = TestObserver::new();

        let hosts = vec![HostMetadata {
            name: "fail-host".to_string(),
            fqdn: "fail-host.example.com".to_string(),
            phase: None,
            tags: vec![],
            reboot_policy: crate::nix::RebootPolicy::Never,
        }];

        let config = PushConfiguration {
            mode: DeploymentMode::Switch,
            force: false,
            concurrency: 1,
            ignore_phases: false,
            quiet: false,
            ssh_user: None,
            allow_reboot: true,
        };

        let result = {
            let push_ctx = PushContext::new(&nix_backend, &deployer, &reboot_detector, &config);
            push(&flake, &push_ctx, hosts, &observer)
        };

        // Should fail during build
        assert!(result.is_err());

        let events = observer.get_events();

        // Phase lifecycle events should be present
        assert!(events.contains(&"phase_start:default:1".to_string()));
        assert!(events.contains(&"phase_complete:default:failed".to_string()));

        // Build events should be present (parallel-per-host model)
        assert!(events.contains(&"build_start:fail-host".to_string()));
        assert!(events.contains(&"build_complete:fail-host:fail".to_string()));

        // NO copy/activate/reboot events should fire (build failed before these steps)
        assert!(!events.iter().any(|e| e.starts_with("copy_")));
        assert!(!events.iter().any(|e| e.starts_with("deploy_")));
        assert!(!events.iter().any(|e| e.starts_with("reboot_")));
    }

    #[test]
    fn test_push_copy_failure_stops_deployment() {
        // Copy fails for one host after successful build
        let flake = Flake::test_context();
        let nix_backend = MockNixBackend::new();
        let deployer =
            MockDeployBackend::new().with_copy_failures(vec!["fail-copy.example.com".to_string()]);
        let reboot_detector = SystemRebootBackend::default();
        let observer = TestObserver::new();

        let hosts = vec![HostMetadata {
            name: "fail-copy".to_string(),
            fqdn: "fail-copy.example.com".to_string(),
            phase: None,
            tags: vec![],
            reboot_policy: crate::nix::RebootPolicy::Never,
        }];

        let config = PushConfiguration {
            mode: DeploymentMode::Switch,
            force: false,
            concurrency: 1,
            ignore_phases: false,
            quiet: false,
            ssh_user: None,
            allow_reboot: true,
        };

        let result = {
            let push_ctx = PushContext::new(&nix_backend, &deployer, &reboot_detector, &config);
            push(&flake, &push_ctx, hosts, &observer)
        };

        // Should fail during copy
        assert!(result.is_err());

        let events = observer.get_events();

        // Phase lifecycle events should be present
        assert!(events.contains(&"phase_start:default:1".to_string()));
        assert!(events.contains(&"phase_complete:default:failed".to_string()));

        // Build should succeed (parallel-per-host model)
        assert!(events.contains(&"build_start:fail-copy".to_string()));
        assert!(events.contains(&"build_complete:fail-copy:ok".to_string()));

        // Copy should fail
        assert!(events.contains(&"copy_start:fail-copy".to_string()));
        assert!(events.contains(&"copy_complete:fail-copy:fail".to_string()));

        // NO activate/reboot events (copy failed before these steps)
        assert!(!events.iter().any(|e| e.starts_with("deploy_")));
        assert!(!events.iter().any(|e| e.starts_with("reboot_")));
    }

    #[test]
    fn test_push_copy_observer_hooks_called() {
        // Verify copy observer hooks fire in correct order
        let flake = Flake::test_context();
        let nix_backend = MockNixBackend::new();
        let deployer = MockDeployBackend::new();
        let reboot_detector = SystemRebootBackend::default();
        let observer = TestObserver::new();

        let hosts = vec![
            HostMetadata {
                name: "host1".to_string(),
                fqdn: "host1.example.com".to_string(),
                phase: Some("p1".to_string()),
                tags: vec![],
                reboot_policy: crate::nix::RebootPolicy::Never,
            },
            HostMetadata {
                name: "host2".to_string(),
                fqdn: "host2.example.com".to_string(),
                phase: Some("p1".to_string()),
                tags: vec![],
                reboot_policy: crate::nix::RebootPolicy::Never,
            },
        ];

        let config = PushConfiguration {
            mode: DeploymentMode::Switch,
            force: false,
            concurrency: 2,
            ignore_phases: false,
            quiet: false,
            ssh_user: None,
            allow_reboot: true,
        };

        {
            let push_ctx = PushContext::new(&nix_backend, &deployer, &reboot_detector, &config);
            push(&flake, &push_ctx, hosts, &observer)
        }
        .unwrap();

        let events = observer.get_events();

        // In parallel-per-host model, verify each host completes its copy
        let copy1_start_idx = events.iter().position(|e| e == "copy_start:host1").unwrap();
        let copy1_complete_idx = events
            .iter()
            .position(|e| e == "copy_complete:host1:ok")
            .unwrap();
        let copy2_start_idx = events.iter().position(|e| e == "copy_start:host2").unwrap();
        let copy2_complete_idx = events
            .iter()
            .position(|e| e == "copy_complete:host2:ok")
            .unwrap();

        // Each host's copy_start should come before its copy_complete
        assert!(copy1_start_idx < copy1_complete_idx);
        assert!(copy2_start_idx < copy2_complete_idx);
    }

    #[test]
    fn test_push_build_observer_hooks_called() {
        // Verify build observer hooks fire in correct order
        let flake = Flake::test_context();
        let nix_backend = MockNixBackend::new();
        let deployer = MockDeployBackend::new();
        let reboot_detector = SystemRebootBackend::default();
        let observer = TestObserver::new();

        let hosts = vec![
            HostMetadata {
                name: "host1".to_string(),
                fqdn: "host1.example.com".to_string(),
                phase: Some("p1".to_string()),
                tags: vec![],
                reboot_policy: crate::nix::RebootPolicy::Never,
            },
            HostMetadata {
                name: "host2".to_string(),
                fqdn: "host2.example.com".to_string(),
                phase: Some("p1".to_string()),
                tags: vec![],
                reboot_policy: crate::nix::RebootPolicy::Never,
            },
        ];

        let config = PushConfiguration {
            mode: DeploymentMode::Switch,
            force: false,
            concurrency: 2,
            ignore_phases: false,
            quiet: false,
            ssh_user: None,
            allow_reboot: true,
        };

        {
            let push_ctx = PushContext::new(&nix_backend, &deployer, &reboot_detector, &config);
            push(&flake, &push_ctx, hosts, &observer)
        }
        .unwrap();

        let events = observer.get_events();

        // In parallel-per-host model, verify each host completes its build
        let build1_start_idx = events
            .iter()
            .position(|e| e == "build_start:host1")
            .unwrap();
        let build1_complete_idx = events
            .iter()
            .position(|e| e == "build_complete:host1:ok")
            .unwrap();
        let build2_start_idx = events
            .iter()
            .position(|e| e == "build_start:host2")
            .unwrap();
        let build2_complete_idx = events
            .iter()
            .position(|e| e == "build_complete:host2:ok")
            .unwrap();

        // Each host's build_start should come before its build_complete
        assert!(build1_start_idx < build1_complete_idx);
        assert!(build2_start_idx < build2_complete_idx);
    }

    #[test]
    fn test_push_build_partial_failure() {
        // Build fails for host 2 of 2 in a phase
        let flake = Flake::test_context();
        let nix_backend = MockNixBackend::new().with_build_failures(vec!["host2".to_string()]);
        let deployer = MockDeployBackend::new();
        let reboot_detector = SystemRebootBackend::default();
        let observer = TestObserver::new();

        let hosts = vec![
            HostMetadata {
                name: "host1".to_string(),
                fqdn: "host1.example.com".to_string(),
                phase: Some("p1".to_string()),
                tags: vec![],
                reboot_policy: crate::nix::RebootPolicy::Never,
            },
            HostMetadata {
                name: "host2".to_string(),
                fqdn: "host2.example.com".to_string(),
                phase: Some("p1".to_string()),
                tags: vec![],
                reboot_policy: crate::nix::RebootPolicy::Never,
            },
        ];

        let config = PushConfiguration {
            mode: DeploymentMode::Switch,
            force: false,
            concurrency: 2,
            ignore_phases: false,
            quiet: false,
            ssh_user: None,
            allow_reboot: true,
        };

        let result = {
            let push_ctx = PushContext::new(&nix_backend, &deployer, &reboot_detector, &config);
            push(&flake, &push_ctx, hosts, &observer)
        };

        assert!(result.is_err());

        let events = observer.get_events();

        // Parallel-per-host model: host1 can proceed even if host2 fails
        // host1 build should succeed
        assert!(events.contains(&"build_complete:host1:ok".to_string()));

        // host2 build should fail
        assert!(events.contains(&"build_complete:host2:fail".to_string()));

        // In parallel model, host1 continues through the pipeline even though host2 failed
        // host1 should complete copy, activate, etc.
        assert!(events
            .iter()
            .any(|e| e.starts_with("copy_") && e.contains("host1")));

        // host2 should NOT proceed to copy (it failed build)
        assert!(!events
            .iter()
            .any(|e| e.starts_with("copy_") && e.contains("host2")));
    }

    #[test]
    fn test_push_ignore_phases_flag() {
        // Test that ignore_phases groups all hosts into "_all" phase
        let flake = Flake::test_context();
        let nix_backend = MockNixBackend::new();
        let deployer = MockDeployBackend::new();
        let reboot_detector = SystemRebootBackend::default();
        let observer = TestObserver::new();

        let hosts = vec![
            HostMetadata {
                name: "db1".to_string(),
                fqdn: "db1.example.com".to_string(),
                phase: Some("database".to_string()),
                tags: vec![],
                reboot_policy: crate::nix::RebootPolicy::Never,
            },
            HostMetadata {
                name: "web1".to_string(),
                fqdn: "web1.example.com".to_string(),
                phase: Some("web".to_string()),
                tags: vec![],
                reboot_policy: crate::nix::RebootPolicy::Never,
            },
        ];

        let config = PushConfiguration {
            mode: DeploymentMode::Switch,
            force: false,
            concurrency: 2,
            ignore_phases: true, // Ignore phase ordering
            quiet: false,
            ssh_user: None,
            allow_reboot: true,
        };

        let results = {
            let push_ctx = PushContext::new(&nix_backend, &deployer, &reboot_detector, &config);
            push(&flake, &push_ctx, hosts, &observer)
        }
        .unwrap();

        assert_eq!(results.success_count(), 2);

        let events = observer.get_events();

        // Both hosts should be in "_all" phase (parallel-per-host model)
        assert!(events.contains(&"phase_start:_all:2".to_string()));

        // Should NOT have separate database/web phases
        assert!(!events.iter().any(|e| e.contains(":database:")));
        assert!(!events.iter().any(|e| e.contains(":web:")));
    }

    #[test]
    fn test_push_phase_failure_stops_next_phase() {
        // Phase 1 activation fails, Phase 2 should never start
        let flake = Flake::test_context();
        let nix_backend = MockNixBackend::new();
        let deployer = MockDeployBackend::new()
            .with_activate_failures(vec!["phase1-host.example.com".to_string()]);
        let reboot_detector = SystemRebootBackend::default();
        let observer = TestObserver::new();

        let hosts = vec![
            HostMetadata {
                name: "phase1-host".to_string(),
                fqdn: "phase1-host.example.com".to_string(),
                phase: Some("phase1".to_string()),
                tags: vec![],
                reboot_policy: crate::nix::RebootPolicy::Never,
            },
            HostMetadata {
                name: "phase2-host".to_string(),
                fqdn: "phase2-host.example.com".to_string(),
                phase: Some("phase2".to_string()),
                tags: vec![],
                reboot_policy: crate::nix::RebootPolicy::Never,
            },
        ];

        let config = PushConfiguration {
            mode: DeploymentMode::Switch,
            force: false,
            concurrency: 1,
            ignore_phases: false,
            quiet: false,
            ssh_user: None,
            allow_reboot: true,
        };

        // When activation fails, push() returns Err but observer events still fire
        let result = {
            let push_ctx = PushContext::new(&nix_backend, &deployer, &reboot_detector, &config);
            push(&flake, &push_ctx, hosts, &observer)
        };

        // Push should fail due to phase1 activation failure
        assert!(
            result.is_err(),
            "Push should return error when activation fails"
        );

        let events = observer.get_events();

        // Phase1 should have all events (build, copy, deploy)
        assert!(events.iter().any(|e| e.contains("phase1")));
        assert!(events.iter().any(|e| e.contains("phase1-host")));

        // Phase2 should have NO events (build, copy, or activate)
        assert!(
            !events.iter().any(|e| e.contains("phase2")),
            "Phase2 should not start after phase1 failure"
        );
        assert!(
            !events.iter().any(|e| e.contains("phase2-host")),
            "Phase2 host should not be processed"
        );
    }

    #[test]
    fn test_push_reboot_execution_success() {
        // Test that reboot is executed when required and observer hooks are called
        let flake = Flake::test_context();
        let nix_backend = MockNixBackend::new();
        let deployer = MockDeployBackend::new();

        // Configure reboot detection to require reboot
        use crate::backends::mock::MockRebootBackend;
        use crate::backends::reboot::RebootReason;
        let mut reboot_required = std::collections::HashMap::new();
        reboot_required.insert(
            "reboot-host.example.com".to_string(),
            RebootReason::KernelChanged,
        );
        let reboot_backend = MockRebootBackend::new().with_reboot_required(reboot_required.clone());

        let observer = TestObserver::new();

        let hosts = vec![make_host_metadata(
            "reboot-host",
            Some("default"),
            crate::nix::RebootPolicy::Auto,
        )];

        let config = PushConfiguration {
            mode: DeploymentMode::Auto, // Auto mode will detect reboot requirement
            force: false,
            concurrency: 1,
            ignore_phases: false,
            quiet: false,
            ssh_user: None,
            allow_reboot: true,
        };

        let results = {
            let push_ctx = PushContext::new(&nix_backend, &deployer, &reboot_backend, &config);
            push(&flake, &push_ctx, hosts, &observer)
        }
        .unwrap();

        // Should succeed
        assert!(results.all_succeeded());
        assert_eq!(results.success_count(), 1);
        assert_eq!(results.failure_count(), 0);

        // Check observer events - should include reboot events
        let events = observer.get_events();

        // Should have reboot events
        assert!(
            events.iter().any(|e| e == "reboot_start:reboot-host"),
            "Missing reboot_start event. Events: {:?}",
            events
        );
        assert!(
            events.iter().any(|e| e == "reboot_complete:reboot-host:ok"),
            "Missing reboot_complete event. Events: {:?}",
            events
        );

        // Verify event sequence: build -> copy -> deploy -> reboot
        let reboot_start_idx = events
            .iter()
            .position(|e| e == "reboot_start:reboot-host")
            .unwrap();
        let deploy_complete_idx = events
            .iter()
            .position(|e| e.starts_with("deploy_complete:reboot-host"))
            .unwrap();

        assert!(
            deploy_complete_idx < reboot_start_idx,
            "Reboot should happen after deployment"
        );
    }

    #[test]
    fn test_push_reboot_execution_failure() {
        // Test that reboot failures are properly handled and reported
        let flake = Flake::test_context();
        let nix_backend = MockNixBackend::new();
        let deployer = MockDeployBackend::new();

        // Configure reboot to be required but fail during execution
        use crate::backends::mock::MockRebootBackend;
        use crate::backends::reboot::RebootReason;
        let mut reboot_required = std::collections::HashMap::new();
        reboot_required.insert(
            "fail-reboot.example.com".to_string(),
            RebootReason::InitrdChanged,
        );
        let reboot_backend = MockRebootBackend::new()
            .with_reboot_required(reboot_required)
            .with_reboot_failures(vec!["fail-reboot.example.com".to_string()]);

        let observer = TestObserver::new();

        let hosts = vec![make_host_metadata(
            "fail-reboot",
            Some("default"),
            crate::nix::RebootPolicy::Auto,
        )];

        let config = PushConfiguration {
            mode: DeploymentMode::Auto,
            force: false,
            concurrency: 1,
            ignore_phases: false,
            quiet: false,
            ssh_user: None,
            allow_reboot: true,
        };

        let results = {
            let push_ctx = PushContext::new(&nix_backend, &deployer, &reboot_backend, &config);
            push(&flake, &push_ctx, hosts, &observer)
        }
        .unwrap();

        // Should report failure due to reboot failure
        assert!(!results.all_succeeded());
        assert_eq!(results.failure_count(), 1);

        // Check that deployment itself succeeded but reboot failed
        let (hostname, result) = &results.results[0];
        assert_eq!(hostname, "fail-reboot");
        assert!(!result.success);
        assert!(
            result.message.contains("reboot failed"),
            "Error should mention reboot failure: {}",
            result.message
        );

        // Check observer events
        let events = observer.get_events();

        // Should have both reboot_start and reboot_complete:fail
        assert!(
            events.iter().any(|e| e == "reboot_start:fail-reboot"),
            "Missing reboot_start event"
        );
        assert!(
            events
                .iter()
                .any(|e| e == "reboot_complete:fail-reboot:fail"),
            "Missing reboot_complete:fail event"
        );
    }

    #[test]
    fn test_push_no_reboot_when_not_required() {
        // Test that reboot hooks are NOT called when reboot is not required
        let flake = Flake::test_context();
        let nix_backend = MockNixBackend::new();
        let deployer = MockDeployBackend::new();

        // No reboot required (empty HashMap)
        use crate::backends::mock::MockRebootBackend;
        let reboot_backend = MockRebootBackend::new();

        let observer = TestObserver::new();

        let hosts = vec![make_host_metadata(
            "no-reboot",
            Some("default"),
            crate::nix::RebootPolicy::Auto,
        )];

        let config = PushConfiguration {
            mode: DeploymentMode::Auto,
            force: false,
            concurrency: 1,
            ignore_phases: false,
            quiet: false,
            ssh_user: None,
            allow_reboot: true,
        };

        let results = {
            let push_ctx = PushContext::new(&nix_backend, &deployer, &reboot_backend, &config);
            push(&flake, &push_ctx, hosts, &observer)
        }
        .unwrap();

        // Should succeed
        assert!(results.all_succeeded());
        assert_eq!(results.success_count(), 1);

        // Check observer events - should NOT include reboot events
        let events = observer.get_events();

        // Should NOT have reboot events
        assert!(
            !events.iter().any(|e| e.contains("reboot_start")),
            "Should not have reboot_start when reboot not required"
        );
        assert!(
            !events.iter().any(|e| e.contains("reboot_complete")),
            "Should not have reboot_complete when reboot not required"
        );

        // Should have completed deployment
        assert!(events.iter().any(|e| e.starts_with("deploy_complete")));
    }

    #[test]
    fn test_push_reboot_with_boot_mode() {
        // Test explicit Boot mode with reboot required
        let flake = Flake::test_context();
        let nix_backend = MockNixBackend::new();
        let deployer = MockDeployBackend::new();

        // Configure reboot required
        use crate::backends::mock::MockRebootBackend;
        use crate::backends::reboot::RebootReason;
        let mut reboot_required = std::collections::HashMap::new();
        reboot_required.insert(
            "boot-host.example.com".to_string(),
            RebootReason::InitChanged,
        );
        let reboot_backend = MockRebootBackend::new().with_reboot_required(reboot_required);

        let observer = TestObserver::new();

        let hosts = vec![make_host_metadata(
            "boot-host",
            Some("default"),
            crate::nix::RebootPolicy::Auto,
        )];

        let config = PushConfiguration {
            mode: DeploymentMode::Boot, // Explicit Boot mode
            force: false,
            concurrency: 1,
            ignore_phases: false,
            quiet: false,
            ssh_user: None,
            allow_reboot: true,
        };

        let results = {
            let push_ctx = PushContext::new(&nix_backend, &deployer, &reboot_backend, &config);
            push(&flake, &push_ctx, hosts, &observer)
        }
        .unwrap();

        // Should succeed
        assert!(results.all_succeeded());

        // Check that reboot was executed
        let events = observer.get_events();
        assert!(
            events.iter().any(|e| e.starts_with("reboot_start")),
            "Boot mode with reboot required should execute reboot"
        );
        assert!(events.iter().any(|e| e.starts_with("reboot_complete")));
    }

    #[test]
    fn test_push_reboot_policy_always() {
        // Test that RebootPolicy::Always forces reboot even if not detected
        let flake = Flake::test_context();
        let nix_backend = MockNixBackend::new();
        let deployer = MockDeployBackend::new();

        // No reboot detected, but policy is Always
        use crate::backends::mock::MockRebootBackend;
        let reboot_backend = MockRebootBackend::new(); // Empty - no reboot required

        let observer = TestObserver::new();

        let hosts = vec![make_host_metadata(
            "always-reboot",
            Some("default"),
            crate::nix::RebootPolicy::Always, // Always reboot
        )];

        let config = PushConfiguration {
            mode: DeploymentMode::Auto,
            force: false,
            concurrency: 1,
            ignore_phases: false,
            quiet: false,
            ssh_user: None,
            allow_reboot: true,
        };

        let results = {
            let push_ctx = PushContext::new(&nix_backend, &deployer, &reboot_backend, &config);
            push(&flake, &push_ctx, hosts, &observer)
        }
        .unwrap();

        // Should succeed
        assert!(results.all_succeeded());

        // Check that reboot was executed despite no detection
        let events = observer.get_events();
        assert!(
            events.iter().any(|e| e == "reboot_start:always-reboot"),
            "RebootPolicy::Always should execute reboot. Events: {:?}",
            events
        );
        assert!(events
            .iter()
            .any(|e| e == "reboot_complete:always-reboot:ok"));
    }

    #[test]
    fn test_push_reboot_policy_never() {
        // Test that RebootPolicy::Never prevents reboot even if detected
        let flake = Flake::test_context();
        let nix_backend = MockNixBackend::new();
        let deployer = MockDeployBackend::new();

        // Reboot required but policy is Never
        use crate::backends::mock::MockRebootBackend;
        use crate::backends::reboot::RebootReason;
        let mut reboot_required = std::collections::HashMap::new();
        reboot_required.insert(
            "never-reboot.example.com".to_string(),
            RebootReason::KernelChanged,
        );
        let reboot_backend = MockRebootBackend::new().with_reboot_required(reboot_required);

        let observer = TestObserver::new();

        let hosts = vec![make_host_metadata(
            "never-reboot",
            Some("default"),
            crate::nix::RebootPolicy::Never, // Never reboot
        )];

        let config = PushConfiguration {
            mode: DeploymentMode::Auto,
            force: false,
            concurrency: 1,
            ignore_phases: false,
            quiet: false,
            ssh_user: None,
            allow_reboot: true,
        };

        let results = {
            let push_ctx = PushContext::new(&nix_backend, &deployer, &reboot_backend, &config);
            push(&flake, &push_ctx, hosts, &observer)
        }
        .unwrap();

        // Should succeed
        assert!(results.all_succeeded());

        // Check that reboot was NOT executed despite detection
        let events = observer.get_events();
        assert!(
            !events.iter().any(|e| e.contains("reboot_start")),
            "RebootPolicy::Never should prevent reboot"
        );
        assert!(!events.iter().any(|e| e.contains("reboot_complete")));

        // Should still have deployed successfully
        assert!(events.iter().any(|e| e.starts_with("deploy_complete")));
    }

    #[test]
    fn test_push_no_reboot_flag_overrides_always_policy() {
        // Test that allow_reboot=false prevents reboot even with RebootPolicy::Always
        let flake = Flake::test_context();
        let nix_backend = MockNixBackend::new();
        let deployer = MockDeployBackend::new();

        // Policy is Always, but user says --no-reboot
        use crate::backends::mock::MockRebootBackend;
        let reboot_backend = MockRebootBackend::new();

        let observer = TestObserver::new();

        let hosts = vec![make_host_metadata(
            "always-reboot",
            Some("default"),
            crate::nix::RebootPolicy::Always, // Policy says always reboot
        )];

        let config = PushConfiguration {
            mode: DeploymentMode::Auto,
            force: false,
            concurrency: 1,
            ignore_phases: false,
            quiet: false,
            ssh_user: None,
            allow_reboot: false, // User says --no-reboot
        };

        let results = {
            let push_ctx = PushContext::new(&nix_backend, &deployer, &reboot_backend, &config);
            push(&flake, &push_ctx, hosts, &observer)
        }
        .unwrap();

        // Should succeed
        assert!(results.all_succeeded());

        // Check that reboot was NOT executed despite policy
        let events = observer.get_events();
        assert!(
            !events.iter().any(|e| e.contains("reboot_start")),
            "allow_reboot=false should prevent reboot even with RebootPolicy::Always"
        );
        assert!(!events.iter().any(|e| e.contains("reboot_complete")));

        // Should still have deployed successfully
        assert!(events.iter().any(|e| e.starts_with("deploy_complete")));
    }

    #[test]
    fn test_push_no_reboot_flag_with_detected_reboot() {
        // Test that allow_reboot=false prevents reboot even when detection says it's needed
        let flake = Flake::test_context();
        let nix_backend = MockNixBackend::new();
        let deployer = MockDeployBackend::new();

        // Reboot detected as required but user says --no-reboot
        use crate::backends::mock::MockRebootBackend;
        use crate::backends::reboot::RebootReason;
        let mut reboot_required = std::collections::HashMap::new();
        reboot_required.insert(
            "needs-reboot.example.com".to_string(),
            RebootReason::KernelChanged,
        );
        let reboot_backend = MockRebootBackend::new().with_reboot_required(reboot_required);

        let observer = TestObserver::new();

        let hosts = vec![make_host_metadata(
            "needs-reboot",
            Some("default"),
            crate::nix::RebootPolicy::Auto, // Auto policy
        )];

        let config = PushConfiguration {
            mode: DeploymentMode::Auto,
            force: false,
            concurrency: 1,
            ignore_phases: false,
            quiet: false,
            ssh_user: None,
            allow_reboot: false, // User says --no-reboot
        };

        let results = {
            let push_ctx = PushContext::new(&nix_backend, &deployer, &reboot_backend, &config);
            push(&flake, &push_ctx, hosts, &observer)
        }
        .unwrap();

        // Should succeed (uses Boot mode but doesn't reboot)
        assert!(results.all_succeeded());

        // Check that reboot was NOT executed despite detection
        let events = observer.get_events();
        assert!(
            !events.iter().any(|e| e.contains("reboot_start")),
            "allow_reboot=false should prevent reboot even when detection says reboot needed"
        );
        assert!(!events.iter().any(|e| e.contains("reboot_complete")));

        // Should still have deployed successfully (Boot mode without reboot)
        assert!(events.iter().any(|e| e.starts_with("deploy_complete")));
    }
}
