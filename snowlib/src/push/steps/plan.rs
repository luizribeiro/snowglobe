use crate::deploy::reboot::{RebootCheck, RebootOps};
use crate::deploy::ActivationMode;
use crate::error::{DeployError, Result, SnowError};
use crate::nix::{DeployTarget, HostMetadata, RebootPolicy, StorePath};
use crate::observer::DeploymentObserver;
use crate::push::config::DeploymentMode;
use crate::push::results::{BlockedHost, DeploymentPlan};
use crate::push::step::{Step, StepResult};
use crate::push::steps::build::BuildOutput;

/// Input for the Planning step (same as BuildOutput/CopyOutput)
pub type PlanningInput = BuildOutput;

/// Output from the Planning step
#[derive(Debug, Clone)]
pub struct PlanningOutput {
    pub plan: DeploymentPlan,
}

/// Planning step - creates deployment plans with reboot detection
pub struct PlanningStep<'a> {
    pub reboot_ops: &'a dyn RebootOps,
    pub mode: DeploymentMode,
    pub force: bool,
}

impl<'a> Step for PlanningStep<'a> {
    type Input = PlanningInput;
    type Output = PlanningOutput;

    fn name(&self) -> &'static str {
        "Planning"
    }

    fn execute_batch(
        &self,
        _phase: &str,
        inputs: Vec<Self::Input>,
        _observer: &dyn DeploymentObserver,
    ) -> Result<Vec<StepResult<Self::Output>>> {
        // Planning step doesn't emit observer notifications

        // Convert inputs to the format expected by create_deployment_plans
        let hosts: Vec<(HostMetadata, DeployTarget, StorePath)> = inputs
            .into_iter()
            .map(|input| (input.metadata, input.target, input.store_path))
            .collect();

        // Create deployment plans with reboot detection
        let (plans, blocked) =
            create_deployment_plans(hosts, self.reboot_ops, self.mode, self.force)?;

        // Handle blocked hosts by returning them as failures
        let mut results = Vec::new();

        for blocked_host in blocked {
            let error = SnowError::Deploy(DeployError::Failed {
                host: blocked_host.hostname.clone(),
                source: std::io::Error::other(blocked_host.reason.clone()),
            });

            results.push(StepResult::failed(blocked_host.hostname, error));
        }

        // Add successful plans
        for plan in plans {
            results.push(StepResult::success(PlanningOutput { plan }));
        }

        // If there are any blocked hosts, we should fail early
        // Find the index of the first failure
        if let Some(fail_index) = results.iter().position(|r| r.is_failed()) {
            // Remove and return the first failure
            let failed = results.remove(fail_index);
            if let StepResult::Failed { error, .. } = failed {
                return Err(error);
            }
        }

        Ok(results)
    }
}

/// Strategy for handling a host deployment
enum ActivationStrategy {
    /// Deploy with the given activation mode and optional reboot check
    Deploy(ActivationMode, Option<RebootCheck>),
    /// Block deployment with the given reason
    Block(String),
}

/// Determine the activation strategy for a single host
fn determine_activation_strategy(
    metadata: &HostMetadata,
    target: &DeployTarget,
    store_path: &StorePath,
    reboot_ops: &dyn RebootOps,
    mode: DeploymentMode,
    force: bool,
) -> Result<ActivationStrategy> {
    match mode {
        DeploymentMode::DryRun => Ok(ActivationStrategy::Deploy(
            ActivationMode::DryActivate,
            None,
        )),
        DeploymentMode::Test => Ok(ActivationStrategy::Deploy(ActivationMode::Test, None)),
        DeploymentMode::Boot => Ok(ActivationStrategy::Deploy(ActivationMode::Boot, None)),
        DeploymentMode::Switch => {
            handle_switch_mode(metadata, target, store_path, reboot_ops, force)
        }
        DeploymentMode::Auto => handle_auto_mode(metadata, target, store_path, reboot_ops),
    }
}

/// Handle switch mode: force immediate activation with safety checks
fn handle_switch_mode(
    metadata: &HostMetadata,
    target: &DeployTarget,
    store_path: &StorePath,
    reboot_ops: &dyn RebootOps,
    force: bool,
) -> Result<ActivationStrategy> {
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
                Some(RebootCheck::required(
                    crate::deploy::reboot::RebootReason::Policy,
                )),
            ));
        }
        RebootPolicy::Auto => {
            // Continue with normal switch mode logic
        }
    }

    let check = reboot_ops.check_reboot_required(target, store_path)?;

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
fn handle_auto_mode(
    metadata: &HostMetadata,
    target: &DeployTarget,
    store_path: &StorePath,
    reboot_ops: &dyn RebootOps,
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
                Some(RebootCheck::required(
                    crate::deploy::reboot::RebootReason::Policy,
                )),
            ));
        }
        RebootPolicy::Auto => {
            // Continue with automatic detection
        }
    }

    let check = reboot_ops.check_reboot_required(target, store_path)?;

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
/// - User-requested deployment mode
/// - Reboot detection results
/// - Force flag
///
/// Returns (plans, blocked_hosts) where blocked_hosts contains hosts that were
/// blocked by safety checks (e.g., --switch without --force when reboot required)
pub fn create_deployment_plans(
    hosts: Vec<(HostMetadata, DeployTarget, StorePath)>,
    reboot_ops: &dyn RebootOps,
    mode: DeploymentMode,
    force: bool,
) -> Result<(Vec<DeploymentPlan>, Vec<BlockedHost>)> {
    let mut plans = Vec::new();
    let mut blocked = Vec::new();

    for (metadata, target, store_path) in hosts {
        match determine_activation_strategy(
            &metadata,
            &target,
            &store_path,
            reboot_ops,
            mode,
            force,
        )? {
            ActivationStrategy::Deploy(activation_mode, reboot_check) => {
                plans.push(DeploymentPlan {
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
