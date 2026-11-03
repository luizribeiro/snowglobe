//! Observer pattern for deployment lifecycle events
//!
//! The observer architecture allows decoupling business logic from side effects
//! like progress reporting, logging, or orchestration hooks.
//!
//! # Design
//!
//! - All methods have default no-op implementations
//! - Observers are infallible - they handle their own errors internally
//! - Trait is `Send + Sync` for use in parallel deployment
//! - Events are emitted at key lifecycle points
//!
//! # Example
//!
//! ```
//! use snowlib::PushObserver;
//!
//! struct MyObserver;
//!
//! impl PushObserver for MyObserver {
//!     fn on_phase_start(&self, phase: &str, hosts: &[String]) {
//!         println!("Starting phase {} with {} hosts", phase, hosts.len());
//!     }
//! }
//! ```

mod log;

pub use log::LogObserver;

use crate::hooks::HookStage;
use crate::push::{HostDeploymentResult, PipelineStep};

/// Something that happened to one host during a push
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostEvent {
    Started(PipelineStep),
    Finished(PipelineStep),
    /// `reason` is the full error chain
    Failed {
        step: PipelineStep,
        reason: String,
    },
    /// Already runs and would boot the built system, so nothing is deployed
    Unchanged,
    WaitingForSerialGroup {
        group: String,
        holder: Option<String>,
    },
    /// The host is through, whatever the outcome: its final result
    Done(HostDeploymentResult),
}

/// Observer trait for push deployment lifecycle events
///
/// All methods have default no-op implementations, so observers only need
/// to implement the events they care about.
///
/// Observers are infallible and should handle errors internally (e.g., by logging).
/// Observer failures will not abort deployment.
pub trait PushObserver: Send + Sync {
    /// Called for every step a host starts, finishes or fails, when it
    /// waits or turns out unchanged, and once when it is done
    fn on_host_event(&self, _hostname: &str, _event: &HostEvent) {}

    /// Called once per phase, before any of its hosts starts
    fn on_phase_start(&self, _phase: &str, _hosts: &[String]) {}

    /// Called once per phase, after all of its hosts are done
    fn on_phase_complete(&self, _phase: &str, _success: bool) {}

    /// Called before running a host's hooks for a stage
    fn on_hooks_start(&self, _hostname: &str, _stage: HookStage) {}

    /// Called after a host's hooks for a stage ran
    fn on_hooks_complete(&self, _hostname: &str, _stage: HookStage, _success: bool) {}

    /// Called before running one of the stage's hooks
    fn on_hook_start(&self, _hostname: &str, _stage: HookStage, _hook_name: &str) {}

    /// Called after one of the stage's hooks ran
    fn on_hook_complete(
        &self,
        _hostname: &str,
        _stage: HookStage,
        _hook_name: &str,
        _success: bool,
    ) {
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
/// # Example
///
/// ```
/// use snowlib::observer::{CompositeObserver, NoopObserver};
/// use snowlib::LogObserver;
///
/// let composite = CompositeObserver::new(vec![
///     Box::new(LogObserver::default()),
///     Box::new(NoopObserver),
/// ]);
/// ```
pub struct CompositeObserver {
    observers: Vec<Box<dyn PushObserver>>,
}

impl CompositeObserver {
    pub fn new(observers: Vec<Box<dyn PushObserver>>) -> Self {
        Self { observers }
    }
}

impl PushObserver for CompositeObserver {
    fn on_host_event(&self, hostname: &str, event: &HostEvent) {
        for observer in &self.observers {
            observer.on_host_event(hostname, event);
        }
    }

    fn on_phase_start(&self, phase: &str, hosts: &[String]) {
        for observer in &self.observers {
            observer.on_phase_start(phase, hosts);
        }
    }

    fn on_phase_complete(&self, phase: &str, success: bool) {
        for observer in &self.observers {
            observer.on_phase_complete(phase, success);
        }
    }

    fn on_hooks_start(&self, hostname: &str, stage: HookStage) {
        for observer in &self.observers {
            observer.on_hooks_start(hostname, stage);
        }
    }

    fn on_hooks_complete(&self, hostname: &str, stage: HookStage, success: bool) {
        for observer in &self.observers {
            observer.on_hooks_complete(hostname, stage, success);
        }
    }

    fn on_hook_start(&self, hostname: &str, stage: HookStage, hook_name: &str) {
        for observer in &self.observers {
            observer.on_hook_start(hostname, stage, hook_name);
        }
    }

    fn on_hook_complete(&self, hostname: &str, stage: HookStage, hook_name: &str, success: bool) {
        for observer in &self.observers {
            observer.on_hook_complete(hostname, stage, hook_name, success);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Default, Clone)]
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
        fn on_host_event(&self, hostname: &str, event: &HostEvent) {
            self.log(format!("{hostname}:{event:?}"));
        }

        fn on_phase_start(&self, phase: &str, hosts: &[String]) {
            self.log(format!("phase_start:{}:{}", phase, hosts.len()));
        }

        fn on_phase_complete(&self, phase: &str, success: bool) {
            self.log(format!("phase_complete:{phase}:{success}"));
        }

        fn on_hooks_start(&self, hostname: &str, stage: HookStage) {
            self.log(format!("hooks_start:{hostname}:{}", stage.nix_name()));
        }

        fn on_hooks_complete(&self, hostname: &str, stage: HookStage, success: bool) {
            self.log(format!(
                "hooks_complete:{hostname}:{}:{success}",
                stage.nix_name()
            ));
        }

        fn on_hook_start(&self, hostname: &str, stage: HookStage, hook_name: &str) {
            self.log(format!(
                "hook_start:{hostname}:{}:{hook_name}",
                stage.nix_name()
            ));
        }

        fn on_hook_complete(
            &self,
            hostname: &str,
            stage: HookStage,
            hook_name: &str,
            success: bool,
        ) {
            self.log(format!(
                "hook_complete:{hostname}:{}:{hook_name}:{success}",
                stage.nix_name()
            ));
        }
    }

    #[test]
    fn test_noop_observer_accepts_every_event() {
        let observer = NoopObserver;
        observer.on_phase_start("p1", &[]);
        observer.on_host_event("cerium", &HostEvent::Started(PipelineStep::Build));
        observer.on_phase_complete("p1", true);
    }

    #[test]
    fn test_composite_passes_every_event_to_each_observer() {
        let first = EventLog::default();
        let second = EventLog::default();
        let composite =
            CompositeObserver::new(vec![Box::new(first.clone()), Box::new(second.clone())]);

        composite.on_phase_start("p3", &["lanthanum".to_string()]);
        composite.on_host_event(
            "lanthanum",
            &HostEvent::Failed {
                step: PipelineStep::Plan,
                reason: "Kernel changed".to_string(),
            },
        );
        composite.on_hooks_start("lanthanum", HookStage::OnFailure);
        composite.on_hook_start("lanthanum", HookStage::OnFailure, "recover-uncordon");
        composite.on_hook_complete("lanthanum", HookStage::OnFailure, "recover-uncordon", true);
        composite.on_hooks_complete("lanthanum", HookStage::OnFailure, true);
        composite.on_phase_complete("p3", false);

        let expected = vec![
            "phase_start:p3:1".to_string(),
            "lanthanum:Failed { step: Plan, reason: \"Kernel changed\" }".to_string(),
            "hooks_start:lanthanum:on-failure".to_string(),
            "hook_start:lanthanum:on-failure:recover-uncordon".to_string(),
            "hook_complete:lanthanum:on-failure:recover-uncordon:true".to_string(),
            "hooks_complete:lanthanum:on-failure:true".to_string(),
            "phase_complete:p3:false".to_string(),
        ];
        assert_eq!(first.get_events(), expected);
        assert_eq!(second.get_events(), expected);
    }
}
