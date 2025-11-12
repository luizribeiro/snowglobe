use crate::deploy::Deployer;
use crate::error::Result;
use crate::observer::DeploymentObserver;
use crate::push::step::{Step, StepResult};
use crate::push::steps::build::BuildOutput;

/// Input for the Copy step (same as BuildOutput)
pub type CopyInput = BuildOutput;

/// Output from the Copy step (passes through BuildOutput with validation)
pub type CopyOutput = BuildOutput;

/// Copy step - copies closures to remote hosts
pub struct CopyStep<'a> {
    pub deployer: &'a dyn Deployer,
}

impl<'a> Step for CopyStep<'a> {
    type Input = CopyInput;
    type Output = CopyOutput;

    fn name(&self) -> &'static str {
        "Copy"
    }

    fn execute_batch(
        &self,
        phase: &str,
        inputs: Vec<Self::Input>,
        observer: &dyn DeploymentObserver,
    ) -> Result<Vec<StepResult<Self::Output>>> {
        // Collect hostnames for phase start notification
        let host_names: Vec<String> = inputs.iter().map(|i| i.metadata.name.clone()).collect();

        // Notify observer of copy step start
        observer.on_copy_step_start(phase, &host_names)?;

        let mut results = Vec::new();

        // Copy closures sequentially
        for input in inputs {
            let hostname = input.metadata.name.clone();

            // Notify observer of copy start
            observer.on_copy_start(&hostname)?;

            // Copy closure to remote host
            match self.deployer.copy_closure(&input.target, &input.store_path) {
                Ok(_) => {
                    // Notify success
                    observer.on_copy_complete(&hostname, true)?;

                    // Pass through the input as output
                    results.push(StepResult::success(input));
                }
                Err(e) => {
                    // Notify failure
                    observer.on_copy_complete(&hostname, false)?;

                    // Copy failure stops deployment immediately
                    return Err(e);
                }
            }
        }

        Ok(results)
    }
}
