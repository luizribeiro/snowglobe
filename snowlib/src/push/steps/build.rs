use crate::backends::deploy::DeployTarget;
use crate::backends::nix::NixBackend;
use crate::error::Result;
use crate::nix::{HostMetadata, SnowContext, StorePath};
use crate::observer::PushObserver;
use crate::push::config::PushConfiguration;
use crate::push::step::{Step, StepResult};

/// Input for the Build step
#[derive(Debug, Clone)]
pub struct BuildInput {
    pub metadata: HostMetadata,
}

/// Output from the Build step
#[derive(Debug, Clone)]
pub struct BuildOutput {
    pub metadata: HostMetadata,
    pub target: DeployTarget,
    pub store_path: StorePath,
}

/// Build step - builds NixOS configurations for hosts
pub struct BuildStep<'a> {
    pub ctx: &'a SnowContext,
    pub nix_backend: &'a dyn NixBackend,
    pub config: &'a PushConfiguration,
}

impl<'a> Step for BuildStep<'a> {
    type Input = BuildInput;
    type Output = BuildOutput;

    fn name(&self) -> &'static str {
        "Build"
    }

    fn execute_batch(
        &self,
        phase: &str,
        inputs: Vec<Self::Input>,
        observer: &dyn PushObserver,
    ) -> Result<Vec<StepResult<Self::Output>>> {
        // Collect hostnames for phase start notification
        let hostnames: Vec<String> = inputs.iter().map(|i| i.metadata.name.clone()).collect();

        // Notify observer of build step start
        observer.on_build_step_start(phase, &hostnames)?;

        let mut results = Vec::new();

        // Build each host sequentially
        let all_succeeded = true;
        for input in inputs {
            let hostname = input.metadata.name.clone();

            // Notify observer of build start
            observer.on_build_start(&hostname)?;

            // Build the host configuration
            match self
                .nix_backend
                .build_host(self.ctx, &hostname, self.config.quiet)
            {
                Ok(store_path) => {
                    // Notify success
                    observer.on_build_complete(&hostname, true)?;

                    // Create deployment target
                    let target = DeployTarget {
                        user: self.config.ssh_user.clone(),
                        hostname: input.metadata.fqdn.clone(),
                        port: None,
                    };

                    // Ensure metadata reflects current phase
                    let mut metadata = input.metadata;
                    metadata.phase = Some(phase.to_string());

                    results.push(StepResult::success(BuildOutput {
                        metadata,
                        target,
                        store_path,
                    }));
                }
                Err(e) => {
                    // Notify failure
                    observer.on_build_complete(&hostname, false)?;

                    // Notify build step completion with failure
                    observer.on_build_step_complete(phase, false)?;

                    // Build failure stops deployment immediately
                    return Err(e);
                }
            }
        }

        // Notify build step completion
        observer.on_build_step_complete(phase, all_succeeded)?;

        Ok(results)
    }
}
