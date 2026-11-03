//! Host deployment pipeline execution
//!
//! This module handles the execution of the deployment pipeline for individual hosts.
//! The pipeline consists of: Build → Copy → Plan → Activate → Reboot
//!
//! The pipeline is fully async, using tokio for concurrent execution of hosts.

use crate::backends::deploy::DeployTarget;
use crate::error::Result;
use crate::hooks::HookStage;
use crate::host::HostMetadata;
use crate::nix::{StorePath, SystemType};
use crate::observer::HostEvent;
use std::collections::HashMap;
use std::future::Future;

use super::config::PushStrategy;
use super::context::PushContext;
use super::planning::create_deployment_plans;
use super::results::{HostDeploymentResult, PipelineStep, PushPlan};
use super::serial::{PhaseScheduler, Section};

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

/// What a failed host's on-failure hooks are told about it
#[derive(Clone, Copy, Default)]
struct FailureContext<'a> {
    target: Option<&'a DeployTarget>,
    store_path: Option<&'a StorePath>,
}

/// The result of a host that stopped early, already reported
type Stopped = HostDeploymentResult;

/// One host's way through the deployment pipeline
struct HostRun<'a, 'ctx> {
    push_ctx: &'a PushContext<'ctx>,
    scheduler: &'a PhaseScheduler,
    metadata: &'a HostMetadata,
}

impl HostRun<'_, '_> {
    fn hostname(&self) -> &str {
        &self.metadata.name
    }

    fn report(&self, event: HostEvent) {
        self.push_ctx
            .observer
            .on_host_event(self.hostname(), &event);
    }

    /// Runs one step, telling the observer when it starts and how it ends
    async fn step<T>(
        &self,
        step: PipelineStep,
        context: FailureContext<'_>,
        work: impl Future<Output = Result<T>>,
    ) -> std::result::Result<T, Stopped> {
        self.report(HostEvent::Started(step));
        match work.await {
            Ok(value) => {
                self.report(HostEvent::Finished(step));
                Ok(value)
            }
            Err(e) => {
                let reason = crate::error::error_chain(&e);
                self.report(HostEvent::Failed {
                    step,
                    reason: reason.clone(),
                });
                Err(self.fail(step, context, reason).await)
            }
        }
    }

    /// Runs a hook stage; hooks report their own progress to the observer
    async fn hooks(
        &self,
        stage: HookStage,
        step: PipelineStep,
        context: FailureContext<'_>,
    ) -> std::result::Result<(), Stopped> {
        match self.run_hooks(stage, context).await {
            Ok(()) => Ok(()),
            Err(e) => Err(self
                .fail(step, context, crate::error::error_chain(&e))
                .await),
        }
    }

    /// Waits until the host's serial group lets it deploy
    async fn enter_serial_section(&self) -> std::result::Result<Section, Stopped> {
        let hostname = self.hostname();
        self.scheduler
            .enter_section(self.metadata, |group, holder| {
                self.report(HostEvent::WaitingForSerialGroup {
                    group: group.to_string(),
                    holder: holder.map(str::to_string),
                });
            })
            .await
            .map_err(|halted| {
                HostDeploymentResult::skipped_by_serial_group(
                    hostname,
                    &halted.group,
                    &halted.failed_host,
                )
            })
    }

    async fn reboot(
        &self,
        plan: &PushPlan,
        context: FailureContext<'_>,
    ) -> std::result::Result<(), Stopped> {
        self.step(
            PipelineStep::Reboot,
            context,
            self.push_ctx.reboot_backend.reboot_and_wait(&plan.target),
        )
        .await
        .map_err(|failure| activated_but_reboot_failed(failure, plan))?;
        self.hooks(HookStage::PostReboot, PipelineStep::Reboot, context)
            .await
    }

    async fn run_hooks(&self, stage: HookStage, context: FailureContext<'_>) -> Result<()> {
        execute_hooks(
            self.push_ctx,
            self.hostname(),
            stage,
            context.target,
            self.metadata.phase.as_deref(),
            context.store_path,
        )
        .await
    }

    async fn fail(
        &self,
        step: PipelineStep,
        context: FailureContext<'_>,
        reason: String,
    ) -> HostDeploymentResult {
        let hostname = self.hostname();
        tracing::error!("{hostname}: failed at {step}: {reason}");
        // Halt first so no other group member starts while on-failure hooks run
        self.scheduler.halt_group_of(self.metadata);

        if let Err(e) = self.run_hooks(HookStage::OnFailure, context).await {
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

    /// Build → (skip if unchanged) → Copy → Plan → pre-deploy hooks →
    /// Activate → Reboot → post-reboot hooks → post-deploy hooks
    ///
    /// Planning happens before any hook so a host blocked by planning is never
    /// disrupted (e.g. drained) for nothing.
    async fn run(&self) -> std::result::Result<HostDeploymentResult, Stopped> {
        let push_ctx = self.push_ctx;
        let hostname = self.hostname();

        let preparation_permit = self.scheduler.permit().await;
        let target = create_target(push_ctx, self.metadata);

        let store_path = self
            .step(
                PipelineStep::Build,
                FailureContext::default(),
                push_ctx.nix_backend.build_host(
                    push_ctx.flake,
                    hostname,
                    self.metadata.system_type,
                    push_ctx.config.output_mode,
                ),
            )
            .await?;
        let built = FailureContext {
            target: None,
            store_path: Some(&store_path),
        };

        if !push_ctx.config.redeploy
            && self
                .step(
                    PipelineStep::Check,
                    built,
                    is_unchanged(push_ctx, hostname, &target, &store_path),
                )
                .await?
        {
            self.report(HostEvent::Unchanged);
            return Ok(HostDeploymentResult::unchanged(hostname));
        }

        self.step(
            PipelineStep::Copy,
            built,
            push_ctx.deploy_backend.copy_closure(&target, &store_path),
        )
        .await?;
        let deployable = FailureContext {
            target: Some(&target),
            store_path: Some(&store_path),
        };

        let plan = self
            .step(
                PipelineStep::Plan,
                deployable,
                plan_host(push_ctx, self.metadata, &target, &store_path),
            )
            .await?;

        drop(preparation_permit);
        let section = self.enter_serial_section().await?;

        self.hooks(HookStage::PreDeploy, PipelineStep::Activate, deployable)
            .await?;

        self.step(
            PipelineStep::Activate,
            deployable,
            push_ctx
                .deploy_backend
                .activate(&target, &store_path, plan.activation_mode),
        )
        .await?;

        let rebooted = should_reboot(push_ctx, &plan);
        if rebooted {
            self.reboot(&plan, deployable).await?;
        }

        self.hooks(HookStage::PostDeploy, PipelineStep::PostDeploy, deployable)
            .await?;
        section.succeeded();

        Ok(HostDeploymentResult {
            hostname: hostname.to_string(),
            success: true,
            failed_at_step: None,
            activation_mode: Some(plan.activation_mode),
            rebooted,
            unchanged: false,
            error_message: None,
        })
    }
}

fn activated_but_reboot_failed(mut failure: Stopped, plan: &PushPlan) -> Stopped {
    failure.activation_mode = Some(plan.activation_mode);
    failure.error_message = failure
        .error_message
        .map(|msg| format!("Deployment succeeded but reboot failed: {}", msg));
    failure
}

/// Execute the full deployment pipeline for a single host
///
/// This function is called concurrently for each host in a phase using tokio tasks.
pub(super) async fn execute_host_pipeline(
    push_ctx: &PushContext<'_>,
    host_metadata: &HostMetadata,
    scheduler: &PhaseScheduler,
) -> Result<HostDeploymentResult> {
    let host = HostRun {
        push_ctx,
        scheduler,
        metadata: host_metadata,
    };
    let result = host.run().await.unwrap_or_else(|stopped| stopped);
    host.report(HostEvent::Done(result.clone()));
    Ok(result)
}

/// Whether the host already runs and would boot `store_path`
///
/// If the installed systems can't be read, the host is deployed rather than
/// skipped. A timeout fails instead: whatever stalled ssh (e.g. a hung
/// ssh-agent) would stall the copy next, and the copy has no time limit.
async fn is_unchanged(
    push_ctx: &PushContext<'_>,
    hostname: &str,
    target: &DeployTarget,
    store_path: &StorePath,
) -> Result<bool> {
    match push_ctx.deploy_backend.host_systems(target).await {
        Ok(systems) => Ok(systems.are(store_path)),
        Err(e) if crate::error::is_timeout(&e) => Err(e),
        Err(e) => {
            tracing::warn!(
                "{}: could not read installed systems, deploying anyway: {}",
                hostname,
                e
            );
            Ok(false)
        }
    }
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

/// Whether the host gets rebooted after activation; Darwin hosts never are
fn should_reboot(push_ctx: &PushContext<'_>, plan: &PushPlan) -> bool {
    push_ctx.config.allow_reboot
        && plan.metadata.system_type == SystemType::NixOS
        && plan.activation_mode == crate::nix::ActivationMode::Boot
        && plan.reboot_check.as_ref().is_none_or(|c| c.required)
}
