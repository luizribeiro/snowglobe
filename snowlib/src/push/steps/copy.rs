use crate::error::Result;
use crate::observer::PushObserver;
use crate::push::context::PushContext;
use crate::push::step::{Step, StepResult};
use crate::push::steps::build::BuildOutput;

/// Input for the Copy step (same as BuildOutput)
pub type CopyInput = BuildOutput;

/// Output from the Copy step (passes through BuildOutput with validation)
pub type CopyOutput = BuildOutput;

/// Copy step - copies closures to remote hosts
pub struct CopyStep<'a> {
    pub push_ctx: &'a PushContext<'a>,
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
        observer: &dyn PushObserver,
    ) -> Result<Vec<StepResult<Self::Output>>> {
        // Collect hostnames for phase start notification
        let hostnames: Vec<String> = inputs.iter().map(|i| i.metadata.name.clone()).collect();

        // Notify observer of copy step start
        observer.on_copy_step_start(phase, &hostnames)?;

        let mut results = Vec::new();

        // Copy closures sequentially
        let all_succeeded = true;
        for input in inputs {
            let hostname = input.metadata.name.clone();

            // Notify observer of copy start
            observer.on_copy_start(&hostname)?;

            // Copy closure to remote host
            match self
                .push_ctx
                .deploy_backend
                .copy_closure(&input.target, &input.store_path)
            {
                Ok(_) => {
                    // Notify success
                    observer.on_copy_complete(&hostname, true)?;

                    // Pass through the input as output
                    results.push(StepResult::success(input));
                }
                Err(e) => {
                    // Notify failure
                    observer.on_copy_complete(&hostname, false)?;

                    // Notify copy step completion with failure
                    observer.on_copy_step_complete(phase, false)?;

                    // Copy failure stops deployment immediately
                    return Err(e);
                }
            }
        }

        // Notify copy step completion
        observer.on_copy_step_complete(phase, all_succeeded)?;

        Ok(results)
    }
}
