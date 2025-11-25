//! Deployment planning and reboot detection
//!
//! This module handles determining the appropriate activation strategy for each host
//! based on push strategy, reboot detection, and reboot policies.

use crate::backends::deploy::DeployTarget;
use crate::backends::reboot::{RebootBackend, RebootCheck, RebootReason};
use crate::error::Result;
use crate::host::HostMetadata;
use crate::nix::{ActivationMode, StorePath};
use crate::reboot::RebootPolicy;

use super::config::PushStrategy;
use super::results::{BlockedHost, PushPlan};

/// Strategy for handling a host deployment
enum ActivationStrategy {
    /// Deploy with the given activation mode and optional reboot check
    Deploy(ActivationMode, Option<RebootCheck>),
    /// Block deployment with the given reason
    Block(String),
}

/// Determine the activation strategy for a single host
async fn determine_activation_strategy(
    metadata: &HostMetadata,
    target: &DeployTarget,
    store_path: &StorePath,
    reboot_backend: &dyn RebootBackend,
    strategy: PushStrategy,
    force: bool,
) -> Result<ActivationStrategy> {
    match strategy {
        PushStrategy::DryRun => Ok(ActivationStrategy::Deploy(
            ActivationMode::DryActivate,
            None,
        )),
        PushStrategy::Test => Ok(ActivationStrategy::Deploy(ActivationMode::Test, None)),
        PushStrategy::Boot => Ok(ActivationStrategy::Deploy(ActivationMode::Boot, None)),
        PushStrategy::Switch => {
            handle_switch_mode(metadata, target, store_path, reboot_backend, force).await
        }
        PushStrategy::Auto => handle_auto_mode(metadata, target, store_path, reboot_backend).await,
    }
}

/// Handle switch mode: force immediate activation with safety checks
async fn handle_switch_mode(
    metadata: &HostMetadata,
    target: &DeployTarget,
    store_path: &StorePath,
    reboot_backend: &dyn RebootBackend,
    force: bool,
) -> Result<ActivationStrategy> {
    // If force is set, override policies and proceed with switch mode
    if !force {
        // Check reboot policy - 'never' and 'always' policies override switch mode
        match metadata.reboot_policy {
            RebootPolicy::Never => {
                // Never policy: use boot mode without reboot
                return Ok(ActivationStrategy::Deploy(
                    ActivationMode::Boot,
                    Some(RebootCheck::not_required()),
                ));
            }
            RebootPolicy::Always => {
                // Always policy: use boot mode and will reboot
                return Ok(ActivationStrategy::Deploy(
                    ActivationMode::Boot,
                    Some(RebootCheck::required(RebootReason::Policy)),
                ));
            }
            RebootPolicy::Auto => {
                // Continue with normal switch mode logic
            }
        }
    }

    let check = reboot_backend
        .check_reboot_required(target, store_path)
        .await?;

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
            ActivationMode::Switch,
            Some(check),
        ))
    }
}

/// Handle auto mode: detect reboot requirement and choose activation mode
async fn handle_auto_mode(
    metadata: &HostMetadata,
    target: &DeployTarget,
    store_path: &StorePath,
    reboot_backend: &dyn RebootBackend,
) -> Result<ActivationStrategy> {
    // Check reboot policy
    match metadata.reboot_policy {
        RebootPolicy::Never => {
            // Never policy: always use boot mode, never reboot
            return Ok(ActivationStrategy::Deploy(
                ActivationMode::Boot,
                Some(RebootCheck::not_required()),
            ));
        }
        RebootPolicy::Always => {
            // Always policy: always use boot mode and always reboot
            return Ok(ActivationStrategy::Deploy(
                ActivationMode::Boot,
                Some(RebootCheck::required(RebootReason::Policy)),
            ));
        }
        RebootPolicy::Auto => {
            // Continue with automatic detection
        }
    }

    let check = reboot_backend
        .check_reboot_required(target, store_path)
        .await?;

    let activation = if check.required {
        ActivationMode::Boot
    } else {
        ActivationMode::Switch
    };

    Ok(ActivationStrategy::Deploy(activation, Some(check)))
}

/// Create deployment plans with reboot detection
///
/// This function determines the appropriate activation mode for each host based on:
/// - User-requested push strategy
/// - Reboot detection results
/// - Force flag
///
/// Returns (plans, blocked_hosts) where blocked_hosts contains hosts that were
/// blocked by safety checks (e.g., --switch without --force when reboot required)
pub async fn create_deployment_plans(
    hosts: Vec<(HostMetadata, DeployTarget, StorePath)>,
    reboot_backend: &dyn RebootBackend,
    strategy: PushStrategy,
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
            strategy,
            force,
        )
        .await?
        {
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
