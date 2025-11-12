use crate::error::{Result, SnowError};
use crate::observer::PushObserver;

/// A step in the deployment pipeline.
///
/// Steps represent individual phases of the deployment process (build, copy, activate, etc.).
/// Each step has typed inputs and outputs, allowing them to be chained together in a pipeline.
///
/// Steps are responsible for:
/// - Executing their operation on a batch of hosts
/// - Emitting observer notifications at appropriate points
/// - Handling errors and producing StepResult for each host
pub trait Step: Send + Sync {
    /// Input type for this step
    type Input: Send;
    /// Output type for this step
    type Output: Send;

    /// Human-readable name for logging/debugging
    fn name(&self) -> &'static str;

    /// Execute step on a batch of inputs.
    ///
    /// Steps emit their own observer notifications during execution.
    ///
    /// # Arguments
    ///
    /// * `phase` - The deployment phase name (for observer notifications)
    /// * `inputs` - Batch of inputs to process
    /// * `observer` - Observer to receive lifecycle events
    ///
    /// # Returns
    ///
    /// Vector of results, one per input. Failed results contain the error,
    /// successful results contain the output for the next step.
    fn execute_batch(
        &self,
        phase: &str,
        inputs: Vec<Self::Input>,
        observer: &dyn PushObserver,
    ) -> Result<Vec<StepResult<Self::Output>>>;
}

/// Result of executing a step on one host
#[derive(Debug)]
pub enum StepResult<T> {
    /// Step succeeded with output
    Success(T),
    /// Step failed with error
    Failed { hostname: String, error: SnowError },
}

impl<T> StepResult<T> {
    /// Create a success result
    pub fn success(output: T) -> Self {
        StepResult::Success(output)
    }

    /// Create a failure result
    pub fn failed(hostname: String, error: SnowError) -> Self {
        StepResult::Failed { hostname, error }
    }

    /// Check if this result is successful
    pub fn is_success(&self) -> bool {
        matches!(self, StepResult::Success(_))
    }

    /// Check if this result is a failure
    pub fn is_failed(&self) -> bool {
        matches!(self, StepResult::Failed { .. })
    }

    /// Get the hostname associated with this result (if failed)
    pub fn hostname(&self) -> Option<&str> {
        match self {
            StepResult::Failed { hostname, .. } => Some(hostname),
            _ => None,
        }
    }
}
