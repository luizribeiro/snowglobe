//! Host deployment pipeline execution
//!
//! This module handles the execution of the deployment pipeline for individual hosts.
//! The pipeline consists of: Build → Copy → Plan → Activate → Reboot
//!
//! The pipeline is fully async, using tokio for concurrent execution of hosts.

use crate::backends::deploy::{DeployResult, DeployTarget};
use crate::error::Result;
use crate::hooks::HookStage;
use crate::host::HostMetadata;
use crate::nix::{StorePath, SystemType};
use std::collections::HashMap;

use super::config::PushStrategy;
use super::context::PushContext;
use super::planning::create_deployment_plans;
use super::results::{HostDeploymentResult, PipelineStep, PushPlan};
use super::serial::PhaseScheduler;

/// Create a deploy target from host metadata and push context
fn create_target(push_ctx: &PushContext<'_>, metadata: &HostMetadata) -> DeployTarget {
    DeployTarget {
        user: push_ctx.config.ssh_user.clone(),
        hostname: metadata.fqdn.clone(),
        port: None,
    }
}

/// Execute hooks for a specific stage
async fn execute_hooks(
    push_ctx: &PushContext<'_>,
    hostname: &str,
    stage: HookStage,
    target: Option<&DeployTarget>,
    phase: Option<&str>,
    store_path: Option<&StorePath>,
) -> Result<()> {
    // A dry run changes nothing, so it must not drain nodes or send alerts
    if push_ctx.config.strategy == PushStrategy::DryRun {
        return Ok(());
    }
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

        let results = executor
            .execute_hooks_for_stage(hostname, stage, target, &env, Some(push_ctx.observer))
            .await?;

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
async fn handle_pipeline_failure(
    push_ctx: &PushContext<'_>,
    scheduler: &PhaseScheduler,
    host_metadata: &HostMetadata,
    target: Option<&DeployTarget>,
    store_path: Option<&StorePath>,
    step: PipelineStep,
    error: crate::error::SnowError,
) -> HostDeploymentResult {
    let hostname = host_metadata.name.as_str();
    let reason = crate::error::error_chain(&error);
    tracing::error!("{hostname}: failed at {step}: {reason}");
    // Halt first so no other group member starts while on-failure hooks run
    scheduler.halt_group_of(host_metadata);

    // Execute on-failure hooks (log errors but don't fail)
    if let Err(e) = execute_hooks(
        push_ctx,
        hostname,
        HookStage::OnFailure,
        target,
        host_metadata.phase.as_deref(),
        store_path,
    )
    .await
    {
        tracing::warn!("On-failure hook execution failed for {}: {}", hostname, e);
    }

    HostDeploymentResult {
        hostname: hostname.to_string(),
        success: false,
        failed_at_step: Some(step),
        activation_mode: None,
        rebooted: false,
        unchanged: false,
        error_message: Some(reason),
    }
}

/// Execute the full deployment pipeline for a single host
///
/// Runs: Build → (skip if unchanged) → Copy → Plan → pre-deploy hooks →
/// Activate → Reboot → post-reboot hooks → post-deploy hooks
///
/// Planning happens before any hook so a host blocked by planning is never
/// disrupted (e.g. drained) for nothing.
///
/// This function is called concurrently for each host in a phase using tokio tasks.
/// Observer callbacks are used to track when hosts enter/leave steps, enabling
/// dynamic step span visibility.
pub(super) async fn execute_host_pipeline(
    push_ctx: &PushContext<'_>,
    host_metadata: &HostMetadata,
    scheduler: &PhaseScheduler,
) -> Result<HostDeploymentResult> {
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
                push_ctx,
                &hostname,
                $stage,
                $target,
                host_metadata.phase.as_deref(),
                $store_path,
            )
            .await
            {
                return Ok(handle_pipeline_failure(
                    push_ctx,
                    scheduler,
                    host_metadata,
                    $target,
                    $store_path,
                    $step,
                    e,
                )
                .await);
            }
        };
    }

    let preparation_permit = scheduler.permit().await;

    // Step 1: Build
    let store_path = match execute_build_step(push_ctx, host_metadata).await {
        Ok(path) => path,
        Err(e) => {
            return Ok(handle_pipeline_failure(
                push_ctx,
                scheduler,
                host_metadata,
                None,
                None,
                PipelineStep::Build,
                e,
            )
            .await);
        }
    };

    if !push_ctx.config.redeploy && is_unchanged(push_ctx, host_metadata, &store_path).await {
        tracing::info!("{}: already running the built system, skipping", hostname);
        push_ctx.observer.on_host_unchanged(&hostname);
        return Ok(HostDeploymentResult::unchanged(hostname));
    }

    // Step 2: Copy
    if let Err(e) = execute_copy_step(push_ctx, host_metadata, &store_path).await {
        return Ok(handle_pipeline_failure(
            push_ctx,
            scheduler,
            host_metadata,
            None,
            Some(&store_path),
            PipelineStep::Copy,
            e,
        )
        .await);
    }

    // Create target for hooks and error handling
    let target = create_target(push_ctx, host_metadata);

    let plan = match plan_host(push_ctx, host_metadata, &target, &store_path).await {
        Ok(plan) => plan,
        Err(e) => {
            return Ok(handle_pipeline_failure(
                push_ctx,
                scheduler,
                host_metadata,
                Some(&target),
                Some(&store_path),
                PipelineStep::Plan,
                e,
            )
            .await);
        }
    };

    drop(preparation_permit);
    let section = match scheduler
        .enter_section(host_metadata, |group, holder| {
            tracing::info!(
                "{}: waiting for serial group '{}' (held by {})",
                hostname,
                group,
                holder.unwrap_or("another host")
            );
            push_ctx
                .observer
                .on_serial_group_wait(&hostname, group, holder);
        })
        .await
    {
        Ok(section) => section,
        Err(halted) => {
            return Ok(HostDeploymentResult::skipped_by_serial_group(
                hostname,
                &halted.group,
                &halted.failed_host,
            ))
        }
    };

    execute_hooks_or_return!(
        HookStage::PreDeploy,
        Some(&plan.target),
        Some(&plan.store_path),
        PipelineStep::Activate
    );

    let activate_result = match execute_activation_step(push_ctx, &plan).await {
        Ok(result) => result,
        Err(e) => {
            return Ok(handle_pipeline_failure(
                push_ctx,
                scheduler,
                host_metadata,
                Some(&plan.target),
                Some(&plan.store_path),
                PipelineStep::Activate,
                e,
            )
            .await);
        }
    };

    // Step 4: Reboot (if needed)
    let rebooted = match execute_reboot_step(push_ctx, &plan).await {
        Ok(rebooted) => rebooted,
        Err(e) => {
            let mut failure = handle_pipeline_failure(
                push_ctx,
                scheduler,
                host_metadata,
                Some(&plan.target),
                Some(&plan.store_path),
                PipelineStep::Reboot,
                e,
            )
            .await;
            failure.activation_mode = Some(plan.activation_mode);
            failure.error_message = failure
                .error_message
                .map(|msg| format!("Deployment succeeded but reboot failed: {}", msg));
            return Ok(failure);
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

    if activate_result.success {
        execute_hooks_or_return!(
            HookStage::PostDeploy,
            Some(&plan.target),
            Some(&plan.store_path),
            PipelineStep::PostDeploy
        );
        section.succeeded();
    }

    // Build success result
    Ok(HostDeploymentResult {
        hostname,
        success: activate_result.success,
        failed_at_step: None,
        activation_mode: Some(plan.activation_mode),
        rebooted,
        unchanged: false,
        error_message: if activate_result.success {
            None
        } else {
            Some(activate_result.message)
        },
    })
}

/// Whether the host already runs and would boot `store_path`
///
/// If the installed systems can't be read, the host is deployed rather than skipped.
async fn is_unchanged(
    push_ctx: &PushContext<'_>,
    host_metadata: &HostMetadata,
    store_path: &StorePath,
) -> bool {
    let target = create_target(push_ctx, host_metadata);
    match push_ctx.deploy_backend.host_systems(&target).await {
        Ok(systems) => systems.are(store_path),
        Err(e) => {
            tracing::warn!(
                "{}: could not read installed systems, deploying anyway: {}",
                host_metadata.name,
                e
            );
            false
        }
    }
}

/// Execute the build step for a single host
///
/// Builds the NixOS configuration and returns the store path.
async fn execute_build_step(
    push_ctx: &PushContext<'_>,
    host_metadata: &HostMetadata,
) -> Result<StorePath> {
    let hostname = &host_metadata.name;

    push_ctx.observer.on_build_start(hostname);
    let store_path = match push_ctx
        .nix_backend
        .build_host(
            push_ctx.flake,
            hostname,
            host_metadata.system_type,
            push_ctx.config.output_mode,
        )
        .await
    {
        Ok(store_path) => store_path,
        Err(e) => {
            push_ctx.observer.on_build_complete(hostname, false);
            return Err(e);
        }
    };
    push_ctx.observer.on_build_complete(hostname, true);

    Ok(store_path)
}

/// Execute the copy step for a single host
///
/// Copies the built closure to the target host.
async fn execute_copy_step(
    push_ctx: &PushContext<'_>,
    host_metadata: &HostMetadata,
    store_path: &StorePath,
) -> Result<()> {
    let hostname = &host_metadata.name;
    let target = create_target(push_ctx, host_metadata);

    push_ctx.observer.on_copy_start(hostname);
    if let Err(e) = push_ctx
        .deploy_backend
        .copy_closure(&target, store_path)
        .await
    {
        push_ctx.observer.on_copy_complete(hostname, false);
        return Err(e);
    }
    push_ctx.observer.on_copy_complete(hostname, true);

    Ok(())
}

/// Choose the activation mode for a host (with reboot detection)
async fn plan_host(
    push_ctx: &PushContext<'_>,
    host_metadata: &HostMetadata,
    target: &DeployTarget,
    store_path: &StorePath,
) -> Result<PushPlan> {
    let (plans, blocked) = create_deployment_plans(
        vec![(host_metadata.clone(), target.clone(), store_path.clone())],
        push_ctx.reboot_backend,
        push_ctx.config.strategy,
        push_ctx.config.force,
    )
    .await?;

    if let Some(blocked) = blocked.into_iter().next() {
        return Err(crate::error::SnowError::Deploy(
            crate::error::DeployError::Failed {
                host: blocked.hostname,
                source: std::io::Error::other(blocked.reason),
            },
        ));
    }

    plans.into_iter().next().ok_or_else(|| {
        crate::error::SnowError::Deploy(crate::error::DeployError::Failed {
            host: host_metadata.name.clone(),
            source: std::io::Error::other("No deployment plan created"),
        })
    })
}

/// Activate the planned system on a host
async fn execute_activation_step(
    push_ctx: &PushContext<'_>,
    plan: &PushPlan,
) -> Result<DeployResult> {
    let hostname = &plan.metadata.name;
    push_ctx.observer.on_activation_start(hostname);
    let result = push_ctx
        .deploy_backend
        .deploy(&plan.target, &plan.store_path, plan.activation_mode)
        .await;
    push_ctx
        .observer
        .on_activation_complete(hostname, result.as_ref().is_ok_and(|r| r.success));
    result
}

/// Execute the reboot step for a single host (if needed)
///
/// Reboots the host if:
/// 1. allow_reboot is true, AND
/// 2. the host runs NixOS (Darwin hosts are never rebooted), AND
/// 3. activation_mode is Boot, AND
/// 4. Either no reboot check OR reboot is required
///
/// Returns whether the host was rebooted
async fn execute_reboot_step(push_ctx: &PushContext<'_>, plan: &PushPlan) -> Result<bool> {
    let hostname = &plan.metadata.name;

    let should_reboot = push_ctx.config.allow_reboot
        && plan.metadata.system_type == SystemType::NixOS
        && plan.activation_mode == crate::nix::ActivationMode::Boot
        && (plan.reboot_check.is_none()
            || plan
                .reboot_check
                .as_ref()
                .map(|c| c.required)
                .unwrap_or(false));

    if !should_reboot {
        return Ok(false);
    }

    push_ctx.observer.on_reboot_start(hostname);
    let result = push_ctx.reboot_backend.reboot_and_wait(&plan.target).await;
    push_ctx
        .observer
        .on_reboot_complete(hostname, result.is_ok());
    result.map(|()| true)
}
