//! Host deployment pipeline execution
//!
//! This module handles the execution of the deployment pipeline for individual hosts.
//! The pipeline consists of: Build → Copy → Plan → Activate → Reboot

use crate::backends::deploy::DeployTarget;
use crate::error::Result;
use crate::nix::{Flake, HostMetadata, StorePath};
use crate::observer::PushObserver;
use std::sync::Arc;

use super::context::PushContext;
use super::planning::create_deployment_plans;

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
                let target = DeployTarget {
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
