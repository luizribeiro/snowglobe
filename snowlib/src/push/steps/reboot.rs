use crate::deploy::reboot::RebootOps;
use crate::deploy::{ActivationMode, DeployResult};
use crate::error::Result;
use crate::observer::DeploymentObserver;
use crate::push::results::DeploymentPlan;
use crate::push::step::{Step, StepResult};

/// Input for the Reboot step
#[derive(Debug, Clone)]
pub struct RebootInput {
    pub plan: DeploymentPlan,
    pub deploy_result: DeployResult,
}

/// Output from the Reboot step
#[derive(Debug, Clone)]
pub struct RebootOutput {
    pub hostname: String,
    pub result: DeployResult, // Updated with reboot status
}

/// Reboot step - reboots hosts that require it
pub struct RebootStep<'a> {
    pub reboot_ops: &'a dyn RebootOps,
    pub allow_reboot: bool,
}

impl<'a> Step for RebootStep<'a> {
    type Input = RebootInput;
    type Output = RebootOutput;

    fn name(&self) -> &'static str {
        "Reboot"
    }

    fn execute_batch(
        &self,
        phase: &str,
        inputs: Vec<Self::Input>,
        observer: &dyn DeploymentObserver,
    ) -> Result<Vec<StepResult<Self::Output>>> {
        // Determine which hosts need reboot
        let hosts_needing_reboot: Vec<String> = inputs
            .iter()
            .filter(|input| {
                self.allow_reboot
                    && input.plan.activation_mode == ActivationMode::Boot
                    && (input.plan.reboot_check.is_none()
                        || input
                            .plan
                            .reboot_check
                            .as_ref()
                            .map(|c| c.required)
                            .unwrap_or(false))
            })
            .map(|input| input.plan.metadata.name.clone())
            .collect();

        // Notify observer of reboot step start (only if there are hosts to reboot)
        if !hosts_needing_reboot.is_empty() {
            observer.on_reboot_step_start(phase, &hosts_needing_reboot)?;
        }

        let mut results = Vec::new();
        let mut all_reboots_succeeded = true;

        for input in inputs {
            let hostname = input.plan.metadata.name.clone();
            let mut result = input.deploy_result.clone();

            // Determine if this host needs reboot
            let should_reboot = hosts_needing_reboot.contains(&hostname);

            if should_reboot {
                // Notify observer of reboot start
                observer.on_reboot_start(&hostname)?;

                // Perform reboot and wait
                match self.reboot_ops.reboot_and_wait(&input.plan.target) {
                    Ok(_) => {
                        // Reboot succeeded
                        observer.on_reboot_complete(&hostname, true)?;
                        result.message = "Deployed & Rebooted".to_string();
                    }
                    Err(e) => {
                        // Reboot failed
                        observer.on_reboot_complete(&hostname, false)?;
                        result.success = false;
                        result.message = format!("Deployed but reboot failed: {}", e);
                        all_reboots_succeeded = false;
                    }
                }
            }

            // Return the result (possibly updated with reboot status)
            results.push(StepResult::success(RebootOutput { hostname, result }));
        }

        // Notify observer of reboot step completion (only if there were hosts to reboot)
        if !hosts_needing_reboot.is_empty() {
            observer.on_reboot_step_complete(phase, all_reboots_succeeded)?;
        }

        Ok(results)
    }
}
