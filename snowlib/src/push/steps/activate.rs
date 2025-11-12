use crate::backends::deploy::{DeployBackend, DeployResult};
use crate::error::{DeployError, Result, SnowError};
use crate::observer::PushObserver;
use crate::push::results::PushPlan;
use crate::push::step::{Step, StepResult};
use rayon::prelude::*;
use std::sync::Arc;

/// Input for the Activate step
#[derive(Debug, Clone)]
pub struct ActivateInput {
    pub plan: PushPlan,
}

/// Output from the Activate step
#[derive(Debug, Clone)]
pub struct ActivateOutput {
    pub hostname: String,
    pub result: DeployResult,
    pub plan: PushPlan, // Pass through for reboot step
}

/// Activate step - activates configurations on remote hosts in parallel
pub struct ActivateStep<'a> {
    pub deploy_backend: &'a dyn DeployBackend,
    pub concurrency: usize,
}

impl<'a> Step for ActivateStep<'a> {
    type Input = ActivateInput;
    type Output = ActivateOutput;

    fn name(&self) -> &'static str {
        "Activate"
    }

    fn execute_batch(
        &self,
        phase: &str,
        inputs: Vec<Self::Input>,
        observer: &dyn PushObserver,
    ) -> Result<Vec<StepResult<Self::Output>>> {
        // Collect hostnames for deploy step start notification
        let hostnames: Vec<String> = inputs
            .iter()
            .map(|i| i.plan.metadata.name.clone())
            .collect();

        // Notify observer of deploy step start
        observer.on_deploy_step_start(phase, &hostnames)?;

        // Deploy hosts in parallel
        let results = self.activate_parallel(inputs, observer)?;

        // Check if all deployments succeeded
        let step_success = results.iter().all(|r| r.is_success());

        // Notify observer of deploy step completion
        observer.on_deploy_step_complete(phase, step_success)?;

        // Return all results - let the orchestrator decide how to handle failures
        // This allows collecting results from all hosts even if some failed
        Ok(results)
    }
}

impl<'a> ActivateStep<'a> {
    /// Deploy hosts in parallel with concurrency limit
    fn activate_parallel(
        &self,
        inputs: Vec<ActivateInput>,
        observer: &dyn PushObserver,
    ) -> Result<Vec<StepResult<ActivateOutput>>> {
        let deployer = Arc::new(self.deploy_backend);
        let observer = Arc::new(observer);

        // Create thread pool with concurrency limit
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(self.concurrency)
            .build()
            .map_err(|e| {
                SnowError::Deploy(DeployError::Failed {
                    host: "thread pool".to_string(),
                    source: std::io::Error::other(e.to_string()),
                })
            })?;

        // Deploy in parallel
        let results: Vec<StepResult<ActivateOutput>> = pool.install(|| {
            inputs
                .into_par_iter()
                .map(|input| {
                    let hostname = input.plan.metadata.name.clone();
                    let activation_mode = input.plan.activation_mode;
                    let deployer = Arc::clone(&deployer);
                    let observer = Arc::clone(&observer);

                    // Notify observer of deployment start
                    if let Err(e) = observer.on_deploy_start(&hostname, activation_mode) {
                        return StepResult::failed(hostname, e);
                    }

                    // Perform deployment
                    let result = deployer
                        .deploy(&input.plan.target, &input.plan.store_path, activation_mode)
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

                    // Notify observer of deployment completion
                    if let Err(e) = observer.on_deploy_complete(&hostname, &result) {
                        // If observer fails, we still return the deployment result
                        // but log the observer error in the result message
                        let mut modified_result = result.clone();
                        modified_result.message =
                            format!("{}\nObserver error: {}", modified_result.message, e);

                        return if modified_result.success {
                            StepResult::success(ActivateOutput {
                                hostname,
                                result: modified_result,
                                plan: input.plan,
                            })
                        } else {
                            StepResult::failed(
                                hostname.clone(),
                                SnowError::Deploy(DeployError::Failed {
                                    host: hostname,
                                    source: std::io::Error::other(modified_result.message),
                                }),
                            )
                        };
                    }

                    // Return result
                    if result.success {
                        StepResult::success(ActivateOutput {
                            hostname,
                            result,
                            plan: input.plan,
                        })
                    } else {
                        StepResult::failed(
                            hostname.clone(),
                            SnowError::Deploy(DeployError::Failed {
                                host: hostname,
                                source: std::io::Error::other(result.message),
                            }),
                        )
                    }
                })
                .collect()
        });

        Ok(results)
    }
}
