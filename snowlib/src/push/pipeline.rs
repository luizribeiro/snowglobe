//! Host deployment pipeline execution
//!
//! This module handles the execution of the deployment pipeline for individual hosts.
//! The pipeline consists of: Build → Copy → Plan → Activate → Reboot

use crate::backends::deploy::{DeployResult, DeployTarget};
use crate::error::Result;
use crate::nix::{Flake, HostMetadata, StorePath};
use crate::observer::PushObserver;
use std::sync::Arc;

use super::context::PushContext;
use super::planning::create_deployment_plans;
use super::results::PushPlan;

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
    host_metadata: &HostMetadata,
    flake: Arc<&Flake>,
    push_ctx: Arc<&PushContext>,
    observer: Arc<&dyn PushObserver>,
) -> Result<(String, DeployResult)> {
    let hostname = &host_metadata.name;

    // Deref Arc to get the actual references
    let flake_ref = flake.as_ref();
    let push_ctx_ref = push_ctx.as_ref();
    let observer_ref = observer.as_ref();

    // Step 1: Build
    let build_output = execute_build_step(host_metadata, flake_ref, push_ctx_ref, *observer_ref)?;

    // Step 2: Copy
    let copy_output = execute_copy_step(build_output, push_ctx_ref, *observer_ref)?;

    // Step 3: Activate (includes planning + activation)
    let (plan, activate_result) =
        execute_activation_step(copy_output, push_ctx_ref, *observer_ref)?;

    // Step 4: Reboot (if needed)
    let final_result = execute_reboot_step(&plan, activate_result, push_ctx_ref, *observer_ref)?;

    Ok((hostname.clone(), final_result))
}

/// Execute the build step for a single host
///
/// Builds the NixOS configuration and creates the deployment target.
fn execute_build_step(
    host_metadata: &HostMetadata,
    flake: &Flake,
    push_ctx: &PushContext,
    observer: &dyn PushObserver,
) -> Result<BuildOutput> {
    let hostname = &host_metadata.name;

    observer.on_build_start(hostname)?;
    let build_output = match push_ctx
        .nix_backend
        .build_host(flake, hostname, push_ctx.config.quiet)
    {
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
            observer.on_build_complete(hostname, false)?;
            return Err(e);
        }
    };
    observer.on_build_complete(hostname, true)?;

    Ok(build_output)
}

/// Execute the copy step for a single host
///
/// Copies the built closure to the target host.
fn execute_copy_step(
    build_output: BuildOutput,
    push_ctx: &PushContext,
    observer: &dyn PushObserver,
) -> Result<CopyOutput> {
    let hostname = build_output.metadata.name.clone();

    observer.on_copy_start(&hostname)?;
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
            observer.on_copy_complete(&hostname, false)?;
            return Err(e);
        }
    };
    observer.on_copy_complete(&hostname, true)?;

    Ok(copy_output)
}

/// Execute the activation step for a single host
///
/// Includes deployment planning (with reboot detection) and activation.
fn execute_activation_step(
    copy_output: CopyOutput,
    push_ctx: &PushContext,
    observer: &dyn PushObserver,
) -> Result<(PushPlan, DeployResult)> {
    let hostname = &copy_output.metadata.name;

    // Start the activate observer span before planning so planning work is visible
    observer.on_activation_start(hostname)?;

    // Planning: Determine activation strategy
    let (plans, blocked) = create_deployment_plans(
        vec![(
            copy_output.metadata.clone(),
            copy_output.target.clone(),
            copy_output.store_path.clone(),
        )],
        push_ctx.reboot_backend,
        push_ctx.config.mode,
        push_ctx.config.force,
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
    let activate_result = push_ctx
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
            DeployResult::failure(error_msg)
        });
    observer.on_activation_complete(hostname, &activate_result)?;

    if !activate_result.success {
        return Err(crate::error::SnowError::Deploy(
            crate::error::DeployError::Failed {
                host: hostname.clone(),
                source: std::io::Error::other(activate_result.message.clone()),
            },
        ));
    }

    Ok((plan, activate_result))
}

/// Execute the reboot step for a single host (if needed)
///
/// Reboots the host if:
/// 1. allow_reboot is true, AND
/// 2. activation_mode is Boot, AND
/// 3. Either no reboot check OR reboot is required
fn execute_reboot_step(
    plan: &PushPlan,
    activate_result: DeployResult,
    push_ctx: &PushContext,
    observer: &dyn PushObserver,
) -> Result<DeployResult> {
    let hostname = &plan.metadata.name;

    let should_reboot = push_ctx.config.allow_reboot
        && plan.activation_mode == crate::backends::deploy::ActivationMode::Boot
        && (plan.reboot_check.is_none()
            || plan
                .reboot_check
                .as_ref()
                .map(|c| c.required)
                .unwrap_or(false));

    let final_result = if should_reboot {
        observer.on_reboot_start(hostname)?;

        match push_ctx.reboot_backend.reboot_and_wait(&plan.target) {
            Ok(()) => {
                observer.on_reboot_complete(hostname, true)?;
                activate_result
            }
            Err(e) => {
                observer.on_reboot_complete(hostname, false)?;
                DeployResult::failure(format!("Deployment succeeded but reboot failed: {}", e))
            }
        }
    } else {
        activate_result
    };

    Ok(final_result)
}
