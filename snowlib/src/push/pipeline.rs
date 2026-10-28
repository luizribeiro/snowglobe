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

use super::context::PushContext;
use super::planning::create_deployment_plans;
use super::results::{HostDeploymentResult, PipelineStep, PushPlan};

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
        error_message: Some(error.to_string()),
    }
}

/// Execute the full deployment pipeline for a single host
///
/// Runs: Build → Copy → Plan → Activate → Reboot
///
/// This function is called concurrently for each host in a phase using tokio tasks.
/// Observer callbacks are used to track when hosts enter/leave steps, enabling
/// dynamic step span visibility.
pub async fn execute_host_pipeline(
    push_ctx: &PushContext<'_>,
    host_metadata: &HostMetadata,
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
                    &hostname,
                    $target,
                    host_metadata.phase.as_deref(),
                    $store_path,
                    $step,
                    e,
                )
                .await);
            }
        };
    }

    // Step 1: Build
    let store_path = match execute_build_step(push_ctx, host_metadata).await {
        Ok(path) => path,
        Err(e) => {
            return Ok(handle_pipeline_failure(
                push_ctx,
                &hostname,
                None,
                host_metadata.phase.as_deref(),
                None,
                PipelineStep::Build,
                e,
            )
            .await);
        }
    };

    // Step 2: Copy
    if let Err(e) = execute_copy_step(push_ctx, host_metadata, &store_path).await {
        return Ok(handle_pipeline_failure(
            push_ctx,
            &hostname,
            None,
            host_metadata.phase.as_deref(),
            Some(&store_path),
            PipelineStep::Copy,
            e,
        )
        .await);
    }

    // Create target for hooks and error handling
    let target = create_target(push_ctx, host_metadata);

    // Step 2.5: Pre-activate hooks
    execute_hooks_or_return!(
        HookStage::PreActivate,
        Some(&target),
        Some(&store_path),
        PipelineStep::Activate
    );

    // Step 3: Activate (includes planning + activation)
    let (plan, activate_result) =
        match execute_activation_step(push_ctx, host_metadata, &store_path).await {
            Ok(result) => result,
            Err(e) => {
                return Ok(handle_pipeline_failure(
                    push_ctx,
                    &hostname,
                    Some(&target),
                    host_metadata.phase.as_deref(),
                    Some(&store_path),
                    PipelineStep::Activate,
                    e,
                )
                .await);
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
    let (final_result, rebooted) = match execute_reboot_step(push_ctx, &plan, activate_result).await
    {
        Ok(result) => result,
        Err(e) => {
            return Ok(handle_pipeline_failure(
                push_ctx,
                &hostname,
                Some(&plan.target),
                host_metadata.phase.as_deref(),
                Some(&plan.store_path),
                PipelineStep::Reboot,
                e,
            )
            .await);
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

/// Execute the activation step for a single host
///
/// Includes deployment planning (with reboot detection) and activation.
async fn execute_activation_step(
    push_ctx: &PushContext<'_>,
    host_metadata: &HostMetadata,
    store_path: &StorePath,
) -> Result<(PushPlan, DeployResult)> {
    let hostname = &host_metadata.name;
    let target = create_target(push_ctx, host_metadata);

    // Start the activate observer span before planning so planning work is visible
    push_ctx.observer.on_activation_start(hostname);

    // Planning: Determine activation strategy
    let (plans, blocked) = create_deployment_plans(
        vec![(host_metadata.clone(), target, store_path.clone())],
        push_ctx.reboot_backend,
        push_ctx.config.strategy,
        push_ctx.config.force,
    )
    .await?;

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
    let activate_result = match push_ctx
        .deploy_backend
        .deploy(&plan.target, &plan.store_path, plan.activation_mode)
        .await
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
/// 2. the host runs NixOS (Darwin hosts are never rebooted), AND
/// 3. activation_mode is Boot, AND
/// 4. Either no reboot check OR reboot is required
///
/// Returns (DeployResult, whether_rebooted)
async fn execute_reboot_step(
    push_ctx: &PushContext<'_>,
    plan: &PushPlan,
    activate_result: DeployResult,
) -> Result<(DeployResult, bool)> {
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

    let (final_result, rebooted) = if should_reboot {
        push_ctx.observer.on_reboot_start(hostname);

        match push_ctx.reboot_backend.reboot_and_wait(&plan.target).await {
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
