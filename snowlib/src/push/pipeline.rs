//! Host deployment pipeline execution
//!
//! This module handles the execution of the deployment pipeline for individual hosts.
//! The pipeline consists of: Build → Copy → Plan → Activate → Reboot
//!
//! Note: Currently uses a hybrid sync/async approach during migration. Functions that
//! call async backends use `block_on_async` to bridge the sync Rayon pipeline with
//! async backend implementations.

use crate::backends::deploy::{DeployResult, DeployTarget};
use crate::error::Result;
use crate::hooks::HookStage;
use crate::host::HostMetadata;
use crate::nix::StorePath;
use std::collections::HashMap;
use std::sync::Arc;

use super::context::PushContext;
use super::planning::create_deployment_plans;
use super::results::{HostDeploymentResult, PipelineStep, PushPlan};

/// Bridge sync code to async during migration
///
/// Creates a new tokio runtime to run async code from sync context.
/// This is a temporary measure until the entire pipeline is async.
fn block_on_async<F: std::future::Future>(future: F) -> F::Output {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => handle.block_on(future),
        Err(_) => {
            // No runtime available, create a temporary one
            tokio::runtime::Runtime::new()
                .expect("Failed to create tokio runtime")
                .block_on(future)
        }
    }
}

/// Intermediate data structure for passing build results through the pipeline
#[derive(Debug, Clone)]
struct BuildOutput {
    metadata: HostMetadata,
    target: DeployTarget,
    store_path: StorePath,
}

/// Intermediate data structure for passing copy results through the pipeline
#[derive(Debug, Clone)]
struct CopyOutput {
    metadata: HostMetadata,
    target: DeployTarget,
    store_path: StorePath,
}

/// Execute hooks for a specific stage
fn execute_hooks(
    push_ctx: &PushContext,
    hostname: &str,
    stage: HookStage,
    target: Option<&DeployTarget>,
    phase: Option<&str>,
    store_path: Option<&StorePath>,
) -> Result<()> {
    if let Some(executor) = push_ctx.hook_executor {
        let mut env = HashMap::new();
        env.insert("SNOW_HOSTNAME".to_string(), hostname.to_string());
        if let Some(phase) = phase {
            env.insert("SNOW_PHASE".to_string(), phase.to_string());
        }
        if let Some(store_path) = store_path {
            env.insert(
                "SNOW_STORE_PATH".to_string(),
                store_path.as_str().to_string(),
            );
        }

        let results = executor.execute_hooks_for_stage(hostname, stage, target, &env)?;

        // Check for any failed hooks (even with continue_on_failure=true)
        let failed_hooks: Vec<_> = results
            .iter()
            .enumerate()
            .filter(|(_, r)| !r.success)
            .collect();

        if !failed_hooks.is_empty() {
            tracing::warn!(
                "⚠️  {} hook(s) failed for {} during {:?} stage (continued due to continueOnFailure=true):",
                failed_hooks.len(),
                hostname,
                stage
            );
            for (idx, result) in failed_hooks {
                tracing::warn!(
                    "   - Hook #{} failed with exit code {:?}: {}",
                    idx + 1,
                    result.exit_code,
                    if !result.stderr.is_empty() {
                        result.stderr.lines().next().unwrap_or("(no error message)")
                    } else {
                        "(no error message)"
                    }
                );
            }
        }
    }
    Ok(())
}

/// Helper to handle pipeline failures consistently
fn handle_pipeline_failure(
    push_ctx: &PushContext,
    hostname: &str,
    target: Option<&DeployTarget>,
    phase: Option<&str>,
    store_path: Option<&StorePath>,
    step: PipelineStep,
    error: crate::error::SnowError,
) -> HostDeploymentResult {
    // Execute on-failure hooks (log errors but don't fail)
    if let Err(e) = execute_hooks(
        push_ctx,
        hostname,
        HookStage::OnFailure,
        target,
        phase,
        store_path,
    ) {
        tracing::warn!("On-failure hook execution failed for {}: {}", hostname, e);
    }

    HostDeploymentResult {
        hostname: hostname.to_string(),
        success: false,
        failed_at_step: Some(step),
        activation_mode: None,
        rebooted: false,
        error_message: Some(error.to_string()),
    }
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
pub fn execute_host_pipeline(
    push_ctx: Arc<&PushContext>,
    host_metadata: &HostMetadata,
) -> Result<HostDeploymentResult> {
    // Deref Arc to get the actual reference
    let push_ctx_ref = push_ctx.as_ref();
    let hostname = host_metadata.name.clone();

    // Macro to execute hooks and return early on failure
    //
    // Automatically handles:
    // - Phase/hostname context from outer scope
    // - Error conversion to HostDeploymentResult via handle_pipeline_failure
    // - On-failure hooks execution (triggered by handle_pipeline_failure)
    // - Early return from execute_host_pipeline on hook failure
    macro_rules! execute_hooks_or_return {
        ($stage:expr, $target:expr, $store_path:expr, $step:expr) => {
            if let Err(e) = execute_hooks(
                push_ctx_ref,
                &hostname,
                $stage,
                $target,
                host_metadata.phase.as_deref(),
                $store_path,
            ) {
                return Ok(handle_pipeline_failure(
                    push_ctx_ref,
                    &hostname,
                    $target,
                    host_metadata.phase.as_deref(),
                    $store_path,
                    $step,
                    e,
                ));
            }
        };
    }

    // Step 1: Build
    let build_output = match execute_build_step(push_ctx_ref, host_metadata) {
        Ok(output) => output,
        Err(e) => {
            return Ok(handle_pipeline_failure(
                push_ctx_ref,
                &hostname,
                None,
                host_metadata.phase.as_deref(),
                None,
                PipelineStep::Build,
                e,
            ));
        }
    };

    // Step 2: Copy
    let copy_output = match execute_copy_step(push_ctx_ref, build_output) {
        Ok(output) => output,
        Err(e) => {
            return Ok(handle_pipeline_failure(
                push_ctx_ref,
                &hostname,
                None,
                host_metadata.phase.as_deref(),
                None,
                PipelineStep::Copy,
                e,
            ));
        }
    };

    // Step 2.5: Pre-activate hooks
    execute_hooks_or_return!(
        HookStage::PreActivate,
        Some(&copy_output.target),
        Some(&copy_output.store_path),
        PipelineStep::Activate
    );

    // Step 3: Activate (includes planning + activation)
    let (plan, activate_result) = match execute_activation_step(push_ctx_ref, copy_output.clone()) {
        Ok(result) => result,
        Err(e) => {
            return Ok(handle_pipeline_failure(
                push_ctx_ref,
                &hostname,
                Some(&copy_output.target),
                host_metadata.phase.as_deref(),
                Some(&copy_output.store_path),
                PipelineStep::Activate,
                e,
            ));
        }
    };

    // Step 3.5: Post-activate hooks
    execute_hooks_or_return!(
        HookStage::PostActivate,
        Some(&plan.target),
        Some(&plan.store_path),
        PipelineStep::Activate
    );

    // Step 4: Reboot (if needed)
    let (final_result, rebooted) = match execute_reboot_step(push_ctx_ref, &plan, activate_result) {
        Ok(result) => result,
        Err(e) => {
            return Ok(handle_pipeline_failure(
                push_ctx_ref,
                &hostname,
                Some(&plan.target),
                host_metadata.phase.as_deref(),
                Some(&plan.store_path),
                PipelineStep::Reboot,
                e,
            ));
        }
    };

    // Step 4.5: Post-reboot hooks (only if actually rebooted)
    if rebooted {
        execute_hooks_or_return!(
            HookStage::PostReboot,
            Some(&plan.target),
            Some(&plan.store_path),
            PipelineStep::Reboot
        );
    }

    // Build success result
    Ok(HostDeploymentResult {
        hostname,
        success: final_result.success,
        failed_at_step: None,
        activation_mode: Some(plan.activation_mode),
        rebooted,
        error_message: if final_result.success {
            None
        } else {
            Some(final_result.message)
        },
    })
}

/// Execute the build step for a single host
///
/// Builds the NixOS configuration and creates the deployment target.
fn execute_build_step(push_ctx: &PushContext, host_metadata: &HostMetadata) -> Result<BuildOutput> {
    let hostname = &host_metadata.name;

    push_ctx.observer.on_build_start(hostname);
    let build_output = match push_ctx.nix_backend.build_host(
        push_ctx.flake,
        hostname,
        host_metadata.system_type,
        push_ctx.config.output_mode,
    ) {
        Ok(store_path) => {
            // Create deployment target
            let target = DeployTarget {
                user: push_ctx.config.ssh_user.clone(),
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
            push_ctx.observer.on_build_complete(hostname, false);
            return Err(e);
        }
    };
    push_ctx.observer.on_build_complete(hostname, true);

    Ok(build_output)
}

/// Execute the copy step for a single host
///
/// Copies the built closure to the target host.
fn execute_copy_step(push_ctx: &PushContext, build_output: BuildOutput) -> Result<CopyOutput> {
    let hostname = build_output.metadata.name.clone();

    push_ctx.observer.on_copy_start(&hostname);
    let copy_output = match push_ctx
        .deploy_backend
        .copy_closure(&build_output.target, &build_output.store_path)
    {
        Ok(()) => CopyOutput {
            metadata: build_output.metadata,
            target: build_output.target,
            store_path: build_output.store_path,
        },
        Err(e) => {
            push_ctx.observer.on_copy_complete(&hostname, false);
            return Err(e);
        }
    };
    push_ctx.observer.on_copy_complete(&hostname, true);

    Ok(copy_output)
}

/// Execute the activation step for a single host
///
/// Includes deployment planning (with reboot detection) and activation.
fn execute_activation_step(
    push_ctx: &PushContext,
    copy_output: CopyOutput,
) -> Result<(PushPlan, DeployResult)> {
    let hostname = &copy_output.metadata.name;

    // Start the activate observer span before planning so planning work is visible
    push_ctx.observer.on_activation_start(hostname);

    // Planning: Determine activation strategy (async, bridged to sync)
    let (plans, blocked) = block_on_async(create_deployment_plans(
        vec![(
            copy_output.metadata.clone(),
            copy_output.target.clone(),
            copy_output.store_path.clone(),
        )],
        push_ctx.reboot_backend,
        push_ctx.config.strategy,
        push_ctx.config.force,
    ))?;

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
    let activate_result =
        match push_ctx
            .deploy_backend
            .deploy(&plan.target, &plan.store_path, plan.activation_mode)
        {
            Ok(result) => {
                push_ctx
                    .observer
                    .on_activation_complete(hostname, result.success);
                result
            }
            Err(e) => {
                push_ctx.observer.on_activation_complete(hostname, false);
                return Err(e);
            }
        };

    Ok((plan, activate_result))
}

/// Execute the reboot step for a single host (if needed)
///
/// Reboots the host if:
/// 1. allow_reboot is true, AND
/// 2. activation_mode is Boot, AND
/// 3. Either no reboot check OR reboot is required
///
/// Returns (DeployResult, whether_rebooted)
fn execute_reboot_step(
    push_ctx: &PushContext,
    plan: &PushPlan,
    activate_result: DeployResult,
) -> Result<(DeployResult, bool)> {
    let hostname = &plan.metadata.name;

    let should_reboot = push_ctx.config.allow_reboot
        && plan.activation_mode == crate::nix::ActivationMode::Boot
        && (plan.reboot_check.is_none()
            || plan
                .reboot_check
                .as_ref()
                .map(|c| c.required)
                .unwrap_or(false));

    let (final_result, rebooted) = if should_reboot {
        push_ctx.observer.on_reboot_start(hostname);

        // Async reboot, bridged to sync
        match block_on_async(push_ctx.reboot_backend.reboot_and_wait(&plan.target)) {
            Ok(()) => {
                push_ctx.observer.on_reboot_complete(hostname, true);
                (activate_result, true)
            }
            Err(e) => {
                push_ctx.observer.on_reboot_complete(hostname, false);
                (
                    DeployResult::failure(format!("Deployment succeeded but reboot failed: {}", e)),
                    false,
                )
            }
        }
    } else {
        (activate_result, false)
    };

    Ok((final_result, rebooted))
}
