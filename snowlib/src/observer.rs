//! Observer pattern for deployment lifecycle events
//!
//! The observer architecture allows decoupling business logic from side effects
//! like progress reporting, logging, or orchestration hooks.
//!
//! # Design
//!
//! - All methods have default no-op implementations
//! - Methods return `Result<()>` to allow observers to abort deployment
//! - Trait is `Send + Sync` for use in parallel deployment
//! - Events are emitted at key lifecycle points
//!
//! # Example
//!
//! ```
//! use snowlib::{PushObserver, DeployResult, ActivationMode, Result};
//!
//! struct MyObserver;
//!
//! impl PushObserver for MyObserver {
//!     fn on_phase_start(&self, phase: &str, hosts: &[String]) -> Result<()> {
//!         println!("Starting phase {} with {} hosts", phase, hosts.len());
//!         Ok(())
//!     }
//! }
//! ```

use crate::backends::deploy::{ActivationMode, DeployResult};
use crate::error::Result;

/// Observer trait for push deployment lifecycle events
///
/// All methods have default no-op implementations, so observers only need
/// to implement the events they care about.
///
/// Methods return `Result<()>` to allow observers to signal errors and
/// potentially abort deployment.
pub trait PushObserver: Send + Sync {
    /// Called when starting the build step for a phase
    ///
    /// This is called once per phase at the start of the build step, before building begins for any host.
    ///
    /// # Arguments
    /// - `phase`: The phase name
    /// - `hosts`: Names of all hosts in this phase
    fn on_build_step_start(&self, _phase: &str, _hosts: &[String]) -> Result<()> {
        Ok(())
    }

    /// Called before building configuration for a specific host
    ///
    /// # Arguments
    /// - `hostname`: The host being built
    fn on_build_start(&self, _hostname: &str) -> Result<()> {
        Ok(())
    }

    /// Called after building configuration for a specific host
    ///
    /// # Arguments
    /// - `hostname`: The host that was built
    /// - `success`: Whether the build succeeded
    fn on_build_complete(&self, _hostname: &str, _success: bool) -> Result<()> {
        Ok(())
    }

    /// Called when the build step completes for a phase
    ///
    /// This is called once per phase at the end of the build step, after all builds finish.
    ///
    /// # Arguments
    /// - `phase`: The phase name
    /// - `success`: Whether all builds in the phase succeeded
    fn on_build_step_complete(&self, _phase: &str, _success: bool) -> Result<()> {
        Ok(())
    }

    /// Called when starting the copy step for a phase
    ///
    /// This is called once per phase at the start of the copy step, before copying begins for any host.
    ///
    /// # Arguments
    /// - `phase`: The phase name
    /// - `hosts`: Names of all hosts in this phase
    fn on_copy_step_start(&self, _phase: &str, _hosts: &[String]) -> Result<()> {
        Ok(())
    }

    /// Called before copying closure to a specific host
    ///
    /// # Arguments
    /// - `hostname`: The host being copied to
    fn on_copy_start(&self, _hostname: &str) -> Result<()> {
        Ok(())
    }

    /// Called after copying closure to a specific host
    ///
    /// # Arguments
    /// - `hostname`: The host that was copied to
    /// - `success`: Whether the copy succeeded
    fn on_copy_complete(&self, _hostname: &str, _success: bool) -> Result<()> {
        Ok(())
    }

    /// Called when the copy step completes for a phase
    ///
    /// This is called once per phase at the end of the copy step, after all copies finish.
    ///
    /// # Arguments
    /// - `phase`: The phase name
    /// - `success`: Whether all copies in the phase succeeded
    fn on_copy_step_complete(&self, _phase: &str, _success: bool) -> Result<()> {
        Ok(())
    }

    /// Called when starting to deploy/activate a phase
    ///
    /// This is called once per phase, before any deployments begin.
    ///
    /// # Arguments
    /// - `phase`: The phase name
    /// - `hosts`: Names of all hosts in this phase that will be deployed
    fn on_phase_start(&self, _phase: &str, _hosts: &[String]) -> Result<()> {
        Ok(())
    }

    /// Called when starting the deploy/activate step for a phase
    ///
    /// This is called once per phase at the start of the deploy step, before activation begins for any host.
    ///
    /// # Arguments
    /// - `phase`: The phase name
    /// - `hosts`: Names of all hosts in this phase
    fn on_deploy_step_start(&self, _phase: &str, _hosts: &[String]) -> Result<()> {
        Ok(())
    }

    /// Called before deploying/activating on a specific host
    ///
    /// This is where orchestration hooks (like Kubernetes cordoning) should run.
    ///
    /// # Arguments
    /// - `hostname`: The host being deployed to
    /// - `mode`: The activation mode being used
    fn on_deploy_start(&self, _hostname: &str, _mode: ActivationMode) -> Result<()> {
        Ok(())
    }

    /// Called after deployment/activation completes on a host
    ///
    /// # Arguments
    /// - `hostname`: The host that was deployed to
    /// - `result`: The deployment result (success or failure)
    fn on_deploy_complete(&self, _hostname: &str, _result: &DeployResult) -> Result<()> {
        Ok(())
    }

    /// Called when the deploy/activate step completes for a phase
    ///
    /// This is called once per phase at the end of the deploy step, after all activations finish.
    ///
    /// # Arguments
    /// - `phase`: The phase name
    /// - `success`: Whether all deployments in the phase succeeded
    fn on_deploy_step_complete(&self, _phase: &str, _success: bool) -> Result<()> {
        Ok(())
    }

    /// Called when a deployment phase completes
    ///
    /// This is called once per phase after all deployments finish.
    ///
    /// # Arguments
    /// - `phase`: The phase name
    /// - `success`: Whether all deployments in the phase succeeded
    fn on_phase_complete(&self, _phase: &str, _success: bool) -> Result<()> {
        Ok(())
    }

    /// Called when starting the reboot step for a phase
    ///
    /// This is called once per phase at the start of the reboot step, before rebooting begins for any host.
    ///
    /// # Arguments
    /// - `phase`: The phase name
    /// - `hosts`: Names of all hosts in this phase that will be rebooted
    fn on_reboot_step_start(&self, _phase: &str, _hosts: &[String]) -> Result<()> {
        Ok(())
    }

    /// Called before rebooting a host
    ///
    /// # Arguments
    /// - `hostname`: The host being rebooted
    fn on_reboot_start(&self, _hostname: &str) -> Result<()> {
        Ok(())
    }

    /// Called after reboot completes (or fails)
    ///
    /// # Arguments
    /// - `hostname`: The host that was rebooted
    /// - `success`: Whether the reboot and reconnection succeeded
    fn on_reboot_complete(&self, _hostname: &str, _success: bool) -> Result<()> {
        Ok(())
    }

    /// Called when the reboot step completes for a phase
    ///
    /// This is called once per phase at the end of the reboot step, after all reboots finish.
    ///
    /// # Arguments
    /// - `phase`: The phase name
    /// - `success`: Whether all reboots in the phase succeeded
    fn on_reboot_step_complete(&self, _phase: &str, _success: bool) -> Result<()> {
        Ok(())
    }

    // --- Span management for explicit parenting ---

    /// Set the current phase span to use as parent for step spans
    ///
    /// This should be called when a phase starts to allow step spans
    /// to use the phase span as their explicit parent.
    fn set_phase_span(&self, _span: tracing::Span) {
        // Default no-op - only observers that track spans need to implement this
    }

    /// Pre-create step spans in the desired display order
    ///
    /// This ensures step spans appear in a consistent order (build, copy, activate, reboot)
    /// in progress indicators, regardless of which step hosts enter first.
    fn prepare_step_spans(&self) {
        // Default no-op - only observers that track spans need to implement this
    }

    // --- Span access methods for explicit parenting ---
    // These allow worker threads to get the current step span to use as an explicit parent

    /// Get the current build step span (if any)
    ///
    /// Returns the shared build span that can be used as an explicit parent
    /// for host spans. Returns None if no hosts are currently building.
    fn build_span(&self) -> Option<tracing::Span> {
        None
    }

    /// Get the current copy step span (if any)
    ///
    /// Returns the shared copy span that can be used as an explicit parent
    /// for host spans. Returns None if no hosts are currently copying.
    fn copy_span(&self) -> Option<tracing::Span> {
        None
    }

    /// Get the current activate step span (if any)
    ///
    /// Returns the shared activate span that can be used as an explicit parent
    /// for host spans. Returns None if no hosts are currently activating.
    fn activate_span(&self) -> Option<tracing::Span> {
        None
    }

    /// Get the current reboot step span (if any)
    ///
    /// Returns the shared reboot span that can be used as an explicit parent
    /// for host spans. Returns None if no hosts are currently rebooting.
    fn reboot_span(&self) -> Option<tracing::Span> {
        None
    }
}

/// No-op observer that does nothing
///
/// Useful for tests or when no observation is needed.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopObserver;

impl PushObserver for NoopObserver {}

/// Composite observer that calls multiple observers in sequence
///
/// If any observer returns an error, the chain stops and the error is returned.
///
/// # Example
///
/// ```
/// use snowlib::observer::{CompositeObserver, NoopObserver};
///
/// let observer1 = NoopObserver;
/// let observer2 = NoopObserver;
///
/// let composite = CompositeObserver::new(vec![
///     Box::new(observer1),
///     Box::new(observer2),
/// ]);
/// ```
pub struct CompositeObserver {
    observers: Vec<Box<dyn PushObserver>>,
}

impl CompositeObserver {
    /// Create a new composite observer from a list of observers
    pub fn new(observers: Vec<Box<dyn PushObserver>>) -> Self {
        Self { observers }
    }
}

impl PushObserver for CompositeObserver {
    fn on_build_step_start(&self, phase: &str, hosts: &[String]) -> Result<()> {
        for observer in &self.observers {
            observer.on_build_step_start(phase, hosts)?;
        }
        Ok(())
    }

    fn on_build_start(&self, hostname: &str) -> Result<()> {
        for observer in &self.observers {
            observer.on_build_start(hostname)?;
        }
        Ok(())
    }

    fn on_build_complete(&self, hostname: &str, success: bool) -> Result<()> {
        for observer in &self.observers {
            observer.on_build_complete(hostname, success)?;
        }
        Ok(())
    }

    fn on_build_step_complete(&self, phase: &str, success: bool) -> Result<()> {
        for observer in &self.observers {
            observer.on_build_step_complete(phase, success)?;
        }
        Ok(())
    }

    fn on_copy_step_start(&self, phase: &str, hosts: &[String]) -> Result<()> {
        for observer in &self.observers {
            observer.on_copy_step_start(phase, hosts)?;
        }
        Ok(())
    }

    fn on_copy_start(&self, hostname: &str) -> Result<()> {
        for observer in &self.observers {
            observer.on_copy_start(hostname)?;
        }
        Ok(())
    }

    fn on_copy_complete(&self, hostname: &str, success: bool) -> Result<()> {
        for observer in &self.observers {
            observer.on_copy_complete(hostname, success)?;
        }
        Ok(())
    }

    fn on_copy_step_complete(&self, phase: &str, success: bool) -> Result<()> {
        for observer in &self.observers {
            observer.on_copy_step_complete(phase, success)?;
        }
        Ok(())
    }

    fn on_phase_start(&self, phase: &str, hosts: &[String]) -> Result<()> {
        for observer in &self.observers {
            observer.on_phase_start(phase, hosts)?;
        }
        Ok(())
    }

    fn on_deploy_step_start(&self, phase: &str, hosts: &[String]) -> Result<()> {
        for observer in &self.observers {
            observer.on_deploy_step_start(phase, hosts)?;
        }
        Ok(())
    }

    fn on_deploy_start(&self, hostname: &str, mode: ActivationMode) -> Result<()> {
        for observer in &self.observers {
            observer.on_deploy_start(hostname, mode)?;
        }
        Ok(())
    }

    fn on_deploy_complete(&self, hostname: &str, result: &DeployResult) -> Result<()> {
        for observer in &self.observers {
            observer.on_deploy_complete(hostname, result)?;
        }
        Ok(())
    }

    fn on_deploy_step_complete(&self, phase: &str, success: bool) -> Result<()> {
        for observer in &self.observers {
            observer.on_deploy_step_complete(phase, success)?;
        }
        Ok(())
    }

    fn on_phase_complete(&self, phase: &str, success: bool) -> Result<()> {
        for observer in &self.observers {
            observer.on_phase_complete(phase, success)?;
        }
        Ok(())
    }

    fn on_reboot_step_start(&self, phase: &str, hosts: &[String]) -> Result<()> {
        for observer in &self.observers {
            observer.on_reboot_step_start(phase, hosts)?;
        }
        Ok(())
    }

    fn on_reboot_start(&self, hostname: &str) -> Result<()> {
        for observer in &self.observers {
            observer.on_reboot_start(hostname)?;
        }
        Ok(())
    }

    fn on_reboot_complete(&self, hostname: &str, success: bool) -> Result<()> {
        for observer in &self.observers {
            observer.on_reboot_complete(hostname, success)?;
        }
        Ok(())
    }

    fn on_reboot_step_complete(&self, phase: &str, success: bool) -> Result<()> {
        for observer in &self.observers {
            observer.on_reboot_step_complete(phase, success)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct EventLog {
        events: Arc<Mutex<Vec<String>>>,
    }

    impl EventLog {
        fn log(&self, event: String) {
            self.events.lock().unwrap().push(event);
        }

        fn get_events(&self) -> Vec<String> {
            self.events.lock().unwrap().clone()
        }
    }

    impl PushObserver for EventLog {
        fn on_phase_start(&self, phase: &str, hosts: &[String]) -> Result<()> {
            self.log(format!("phase_start:{}:{}", phase, hosts.len()));
            Ok(())
        }

        fn on_deploy_start(&self, hostname: &str, _mode: ActivationMode) -> Result<()> {
            self.log(format!("deploy_start:{}", hostname));
            Ok(())
        }

        fn on_deploy_complete(&self, hostname: &str, result: &DeployResult) -> Result<()> {
            self.log(format!(
                "deploy_complete:{}:{}",
                hostname,
                if result.success { "ok" } else { "failed" }
            ));
            Ok(())
        }
    }

    #[test]
    fn test_noop_observer() {
        let observer = NoopObserver;
        assert!(observer.on_phase_start("p1", &[]).is_ok());
        assert!(observer
            .on_deploy_start("host1", ActivationMode::Switch)
            .is_ok());
        assert!(observer
            .on_deploy_complete("host1", &DeployResult::success("ok"))
            .is_ok());
    }

    #[test]
    fn test_event_log_observer() {
        let log = EventLog::default();

        log.on_phase_start("p1", &["host1".to_string(), "host2".to_string()])
            .unwrap();
        log.on_deploy_start("host1", ActivationMode::Switch)
            .unwrap();
        log.on_deploy_complete("host1", &DeployResult::success("ok"))
            .unwrap();

        let events = log.get_events();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0], "phase_start:p1:2");
        assert_eq!(events[1], "deploy_start:host1");
        assert_eq!(events[2], "deploy_complete:host1:ok");
    }

    #[test]
    fn test_composite_observer() {
        let log1 = EventLog::default();
        let log2 = EventLog::default();

        let log1_clone = EventLog {
            events: log1.events.clone(),
        };
        let log2_clone = EventLog {
            events: log2.events.clone(),
        };

        let composite = CompositeObserver::new(vec![Box::new(log1_clone), Box::new(log2_clone)]);

        composite
            .on_phase_start("p1", &["host1".to_string()])
            .unwrap();
        composite
            .on_deploy_start("host1", ActivationMode::Switch)
            .unwrap();

        // Both observers should have received events
        assert_eq!(log1.get_events().len(), 2);
        assert_eq!(log2.get_events().len(), 2);
    }

    #[test]
    fn test_observer_error_propagation() {
        struct FailingObserver;
        impl PushObserver for FailingObserver {
            fn on_deploy_start(&self, _hostname: &str, _mode: ActivationMode) -> Result<()> {
                Err(crate::error::SnowError::Deploy(
                    crate::error::DeployError::Failed {
                        host: "test".to_string(),
                        source: std::io::Error::other("observer error"),
                    },
                ))
            }
        }

        let observer = FailingObserver;
        let result = observer.on_deploy_start("host1", ActivationMode::Switch);
        assert!(result.is_err());
    }
}
