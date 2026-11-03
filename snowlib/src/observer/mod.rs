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

/// Translates a host event into the per-step callbacks below, for observers
/// that don't override [`PushObserver::on_host_event`]. Steps without a
/// callback (check, plan, post-deploy, serial group) are dropped.
#[doc(hidden)]
pub fn forward_to_step_callbacks<O: PushObserver + ?Sized>(
    observer: &O,
    hostname: &str,
    event: &HostEvent,
) {
    use PipelineStep::*;
    match event {
        HostEvent::Started(Build) => observer.on_build_start(hostname),
        HostEvent::Started(Copy) => observer.on_copy_start(hostname),
        HostEvent::Started(Activate) => observer.on_activation_start(hostname),
        HostEvent::Started(Reboot) => observer.on_reboot_start(hostname),
        HostEvent::Finished(step) => forward_completion(observer, hostname, *step, true),
        HostEvent::Failed { step, .. } => forward_completion(observer, hostname, *step, false),
        HostEvent::Unchanged => observer.on_host_unchanged(hostname),
        HostEvent::WaitingForSerialGroup { group, holder } => {
            observer.on_serial_group_wait(hostname, group, holder.as_deref())
        }
        HostEvent::Started(_) | HostEvent::Done(_) => {}
    }
}

fn forward_completion<O: PushObserver + ?Sized>(
    observer: &O,
    hostname: &str,
    step: PipelineStep,
    success: bool,
) {
    match step {
        PipelineStep::Build => observer.on_build_complete(hostname, success),
        PipelineStep::Copy => observer.on_copy_complete(hostname, success),
        PipelineStep::Activate => observer.on_activation_complete(hostname, success),
        PipelineStep::Reboot => observer.on_reboot_complete(hostname, success),
        _ => {}
    }
}

/// Observer trait for push deployment lifecycle events
///
/// All methods have default no-op implementations, so observers only need
/// to implement the events they care about.
///
/// Observers are infallible and should handle errors internally (e.g., by logging).
/// Observer failures will not abort deployment.
pub trait PushObserver: Send + Sync {
    /// Called for every step a host starts, finishes or fails, and when it
    /// waits or turns out unchanged
    ///
    /// Overriding this stops the per-step callbacks from being called, unless
    /// the override calls [`forward_to_step_callbacks`] itself.
    fn on_host_event(&self, hostname: &str, event: &HostEvent) {
        forward_to_step_callbacks(self, hostname, event);
    }

    /// Called when starting the build step for a phase
    ///
    /// This is called once per phase at the start of the build step, before building begins for any host.
    ///
    /// # Arguments
    /// - `phase`: The phase name
    /// - `hosts`: Names of all hosts in this phase
    fn on_build_step_start(&self, _phase: &str, _hosts: &[String]) {}

    /// Called before building configuration for a specific host
    ///
    /// # Arguments
    /// - `hostname`: The host being built
    fn on_build_start(&self, _hostname: &str) {}

    /// Called after building configuration for a specific host
    ///
    /// # Arguments
    /// - `hostname`: The host that was built
    /// - `success`: Whether the build succeeded
    fn on_build_complete(&self, _hostname: &str, _success: bool) {}

    /// Called when the build step completes for a phase
    ///
    /// This is called once per phase at the end of the build step, after all builds finish.
    ///
    /// # Arguments
    /// - `phase`: The phase name
    /// - `success`: Whether all builds in the phase succeeded
    fn on_build_step_complete(&self, _phase: &str, _success: bool) {}

    /// Called when starting the copy step for a phase
    ///
    /// This is called once per phase at the start of the copy step, before copying begins for any host.
    ///
    /// # Arguments
    /// - `phase`: The phase name
    /// - `hosts`: Names of all hosts in this phase
    fn on_copy_step_start(&self, _phase: &str, _hosts: &[String]) {}

    /// Called before copying closure to a specific host
    ///
    /// # Arguments
    /// - `hostname`: The host being copied to
    fn on_copy_start(&self, _hostname: &str) {}

    /// Called after copying closure to a specific host
    ///
    /// # Arguments
    /// - `hostname`: The host that was copied to
    /// - `success`: Whether the copy succeeded
    fn on_copy_complete(&self, _hostname: &str, _success: bool) {}

    /// Called when the copy step completes for a phase
    ///
    /// This is called once per phase at the end of the copy step, after all copies finish.
    ///
    /// # Arguments
    /// - `phase`: The phase name
    /// - `success`: Whether all copies in the phase succeeded
    fn on_copy_step_complete(&self, _phase: &str, _success: bool) {}

    /// Called when starting to deploy/activate a phase
    ///
    /// This is called once per phase, before any deployments begin.
    ///
    /// # Arguments
    /// - `phase`: The phase name
    /// - `hosts`: Names of all hosts in this phase that will be deployed
    fn on_phase_start(&self, _phase: &str, _hosts: &[String]) {}

    /// Called when starting the deploy/activate step for a phase
    ///
    /// This is called once per phase at the start of the deploy step, before activation begins for any host.
    ///
    /// # Arguments
    /// - `phase`: The phase name
    /// - `hosts`: Names of all hosts in this phase
    fn on_deploy_step_start(&self, _phase: &str, _hosts: &[String]) {}

    /// Called before activating on a specific host
    ///
    /// This is where orchestration hooks (like Kubernetes cordoning) should run.
    /// This is called before planning and activation begin.
    ///
    /// # Arguments
    /// - `hostname`: The host being activated
    fn on_activation_start(&self, _hostname: &str) {}

    /// Called when a host is skipped after its build because it already runs
    /// and would boot the built system
    fn on_host_unchanged(&self, _hostname: &str) {}

    /// Called when a host has to wait for its serial group before deploying
    ///
    /// `holder` is the group member currently deploying, if known.
    fn on_serial_group_wait(&self, _hostname: &str, _group: &str, _holder: Option<&str>) {}

    /// Called after activation completes on a host
    ///
    /// # Arguments
    /// - `hostname`: The host that was activated
    /// - `success`: Whether activation succeeded
    fn on_activation_complete(&self, _hostname: &str, _success: bool) {}

    /// Called when the deploy/activate step completes for a phase
    ///
    /// This is called once per phase at the end of the deploy step, after all activations finish.
    ///
    /// # Arguments
    /// - `phase`: The phase name
    /// - `success`: Whether all deployments in the phase succeeded
    fn on_deploy_step_complete(&self, _phase: &str, _success: bool) {}

    /// Called when a deployment phase completes
    ///
    /// This is called once per phase after all deployments finish.
    ///
    /// # Arguments
    /// - `phase`: The phase name
    /// - `success`: Whether all deployments in the phase succeeded
    fn on_phase_complete(&self, _phase: &str, _success: bool) {}

    /// Called when starting the reboot step for a phase
    ///
    /// This is called once per phase at the start of the reboot step, before rebooting begins for any host.
    ///
    /// # Arguments
    /// - `phase`: The phase name
    /// - `hosts`: Names of all hosts in this phase that will be rebooted
    fn on_reboot_step_start(&self, _phase: &str, _hosts: &[String]) {}

    /// Called before rebooting a host
    ///
    /// # Arguments
    /// - `hostname`: The host being rebooted
    fn on_reboot_start(&self, _hostname: &str) {}

    /// Called after reboot completes (or fails)
    ///
    /// # Arguments
    /// - `hostname`: The host that was rebooted
    /// - `success`: Whether the reboot and reconnection succeeded
    fn on_reboot_complete(&self, _hostname: &str, _success: bool) {}

    /// Called when the reboot step completes for a phase
    ///
    /// This is called once per phase at the end of the reboot step, after all reboots finish.
    ///
    /// # Arguments
    /// - `phase`: The phase name
    /// - `success`: Whether all reboots in the phase succeeded
    fn on_reboot_step_complete(&self, _phase: &str, _success: bool) {}

    /// Called before executing hooks for a stage on a host
    ///
    /// # Arguments
    /// - `hostname`: The host for which hooks are being executed
    /// - `stage`: The hook stage (PreDeploy, PostDeploy, etc.)
    fn on_hooks_start(&self, _hostname: &str, _stage: HookStage) {}

    /// Called after all hooks for a stage complete on a host
    ///
    /// # Arguments
    /// - `hostname`: The host for which hooks were executed
    /// - `stage`: The hook stage (PreDeploy, PostDeploy, etc.)
    /// - `success`: Whether all hooks succeeded
    fn on_hooks_complete(&self, _hostname: &str, _stage: HookStage, _success: bool) {}

    /// Called before executing a specific hook
    ///
    /// # Arguments
    /// - `hostname`: The host for which the hook is being executed
    /// - `stage`: The hook stage (PreDeploy, PostDeploy, etc.)
    /// - `hook_name`: The name of the hook being executed
    fn on_hook_start(&self, _hostname: &str, _stage: HookStage, _hook_name: &str) {}

    /// Called after a specific hook completes
    ///
    /// # Arguments
    /// - `hostname`: The host for which the hook was executed
    /// - `stage`: The hook stage (PreDeploy, PostDeploy, etc.)
    /// - `hook_name`: The name of the hook that was executed
    /// - `success`: Whether the hook succeeded
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
/// All observers are called even if one encounters an error internally.
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
    fn on_host_event(&self, hostname: &str, event: &HostEvent) {
        for observer in &self.observers {
            observer.on_host_event(hostname, event);
        }
    }

    fn on_build_step_start(&self, phase: &str, hosts: &[String]) {
        for observer in &self.observers {
            observer.on_build_step_start(phase, hosts);
        }
    }

    fn on_build_start(&self, hostname: &str) {
        for observer in &self.observers {
            observer.on_build_start(hostname);
        }
    }

    fn on_build_complete(&self, hostname: &str, success: bool) {
        for observer in &self.observers {
            observer.on_build_complete(hostname, success);
        }
    }

    fn on_build_step_complete(&self, phase: &str, success: bool) {
        for observer in &self.observers {
            observer.on_build_step_complete(phase, success);
        }
    }

    fn on_copy_step_start(&self, phase: &str, hosts: &[String]) {
        for observer in &self.observers {
            observer.on_copy_step_start(phase, hosts);
        }
    }

    fn on_copy_start(&self, hostname: &str) {
        for observer in &self.observers {
            observer.on_copy_start(hostname);
        }
    }

    fn on_copy_complete(&self, hostname: &str, success: bool) {
        for observer in &self.observers {
            observer.on_copy_complete(hostname, success);
        }
    }

    fn on_copy_step_complete(&self, phase: &str, success: bool) {
        for observer in &self.observers {
            observer.on_copy_step_complete(phase, success);
        }
    }

    fn on_phase_start(&self, phase: &str, hosts: &[String]) {
        for observer in &self.observers {
            observer.on_phase_start(phase, hosts);
        }
    }

    fn on_deploy_step_start(&self, phase: &str, hosts: &[String]) {
        for observer in &self.observers {
            observer.on_deploy_step_start(phase, hosts);
        }
    }

    fn on_activation_start(&self, hostname: &str) {
        for observer in &self.observers {
            observer.on_activation_start(hostname);
        }
    }

    fn on_serial_group_wait(&self, hostname: &str, group: &str, holder: Option<&str>) {
        for observer in &self.observers {
            observer.on_serial_group_wait(hostname, group, holder);
        }
    }

    fn on_host_unchanged(&self, hostname: &str) {
        for observer in &self.observers {
            observer.on_host_unchanged(hostname);
        }
    }

    fn on_activation_complete(&self, hostname: &str, success: bool) {
        for observer in &self.observers {
            observer.on_activation_complete(hostname, success);
        }
    }

    fn on_deploy_step_complete(&self, phase: &str, success: bool) {
        for observer in &self.observers {
            observer.on_deploy_step_complete(phase, success);
        }
    }

    fn on_phase_complete(&self, phase: &str, success: bool) {
        for observer in &self.observers {
            observer.on_phase_complete(phase, success);
        }
    }

    fn on_reboot_step_start(&self, phase: &str, hosts: &[String]) {
        for observer in &self.observers {
            observer.on_reboot_step_start(phase, hosts);
        }
    }

    fn on_reboot_start(&self, hostname: &str) {
        for observer in &self.observers {
            observer.on_reboot_start(hostname);
        }
    }

    fn on_reboot_complete(&self, hostname: &str, success: bool) {
        for observer in &self.observers {
            observer.on_reboot_complete(hostname, success);
        }
    }

    fn on_reboot_step_complete(&self, phase: &str, success: bool) {
        for observer in &self.observers {
            observer.on_reboot_step_complete(phase, success);
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

    #[derive(Default)]
    struct EventLog {
        events: Arc<Mutex<Vec<String>>>,
    }

    impl EventLog {
        fn log(&self, event: String) {
            self.events.lock().unwrap().push(event);
        }

        fn log_completion(&self, step: &str, hostname: &str, success: bool) {
            let outcome = if success { "ok" } else { "failed" };
            self.log(format!("{step}_complete:{hostname}:{outcome}"));
        }

        fn get_events(&self) -> Vec<String> {
            self.events.lock().unwrap().clone()
        }
    }

    impl PushObserver for EventLog {
        fn on_phase_start(&self, phase: &str, hosts: &[String]) {
            self.log(format!("phase_start:{}:{}", phase, hosts.len()));
        }

        fn on_build_start(&self, hostname: &str) {
            self.log(format!("build_start:{}", hostname));
        }

        fn on_build_complete(&self, hostname: &str, success: bool) {
            self.log_completion("build", hostname, success);
        }

        fn on_copy_start(&self, hostname: &str) {
            self.log(format!("copy_start:{}", hostname));
        }

        fn on_copy_complete(&self, hostname: &str, success: bool) {
            self.log_completion("copy", hostname, success);
        }

        fn on_activation_start(&self, hostname: &str) {
            self.log(format!("activation_start:{}", hostname));
        }

        fn on_activation_complete(&self, hostname: &str, success: bool) {
            self.log_completion("activation", hostname, success);
        }

        fn on_reboot_start(&self, hostname: &str) {
            self.log(format!("reboot_start:{}", hostname));
        }

        fn on_reboot_complete(&self, hostname: &str, success: bool) {
            self.log_completion("reboot", hostname, success);
        }

        fn on_host_unchanged(&self, hostname: &str) {
            self.log(format!("unchanged:{}", hostname));
        }

        fn on_serial_group_wait(&self, hostname: &str, group: &str, holder: Option<&str>) {
            self.log(format!("serial_wait:{}:{}:{:?}", hostname, group, holder));
        }
    }

    #[derive(Default)]
    struct HostEventLog {
        events: Arc<Mutex<Vec<(String, HostEvent)>>>,
    }

    impl PushObserver for HostEventLog {
        fn on_host_event(&self, hostname: &str, event: &HostEvent) {
            self.events
                .lock()
                .unwrap()
                .push((hostname.to_string(), event.clone()));
        }
    }

    fn failed(step: PipelineStep) -> HostEvent {
        HostEvent::Failed {
            step,
            reason: "ssh: connect to host cerium port 22: Connection timed out".to_string(),
        }
    }

    #[test]
    fn test_host_events_reach_observers_that_only_have_step_callbacks() {
        for (step, name) in [
            (PipelineStep::Build, "build"),
            (PipelineStep::Copy, "copy"),
            (PipelineStep::Activate, "activation"),
            (PipelineStep::Reboot, "reboot"),
        ] {
            let log = EventLog::default();

            log.on_host_event("cerium", &HostEvent::Started(step));
            log.on_host_event("cerium", &failed(step));
            log.on_host_event("radon", &HostEvent::Finished(step));

            assert_eq!(
                log.get_events(),
                vec![
                    format!("{name}_start:cerium"),
                    format!("{name}_complete:cerium:failed"),
                    format!("{name}_complete:radon:ok"),
                ],
                "{step}"
            );
        }
    }

    #[test]
    fn test_unchanged_and_serial_waits_reach_their_callbacks() {
        let log = EventLog::default();

        log.on_host_event("sodium", &HostEvent::Unchanged);
        log.on_host_event(
            "neodymium",
            &HostEvent::WaitingForSerialGroup {
                group: "k8s".to_string(),
                holder: Some("cerium".to_string()),
            },
        );
        log.on_host_event(
            "samarium",
            &HostEvent::WaitingForSerialGroup {
                group: "k8s".to_string(),
                holder: None,
            },
        );

        assert_eq!(
            log.get_events(),
            vec![
                "unchanged:sodium",
                "serial_wait:neodymium:k8s:Some(\"cerium\")",
                "serial_wait:samarium:k8s:None",
            ]
        );
    }

    #[test]
    fn test_steps_without_a_step_callback_are_dropped_when_forwarding() {
        for step in [
            PipelineStep::Check,
            PipelineStep::Plan,
            PipelineStep::PostDeploy,
            PipelineStep::SerialGroup,
        ] {
            let log = EventLog::default();

            log.on_host_event("lanthanum", &HostEvent::Started(step));
            log.on_host_event("lanthanum", &HostEvent::Finished(step));
            log.on_host_event("lanthanum", &failed(step));

            assert!(log.get_events().is_empty(), "{step}");
        }
    }

    #[test]
    fn test_composite_passes_host_events_to_each_observer_unchanged() {
        let host_events = HostEventLog::default();
        let step_events = EventLog::default();
        let composite = CompositeObserver::new(vec![
            Box::new(HostEventLog {
                events: host_events.events.clone(),
            }),
            Box::new(EventLog {
                events: step_events.events.clone(),
            }),
        ]);

        composite.on_host_event("lanthanum", &failed(PipelineStep::Plan));
        composite.on_host_event("radon", &HostEvent::Started(PipelineStep::Activate));

        assert_eq!(
            *host_events.events.lock().unwrap(),
            vec![
                ("lanthanum".to_string(), failed(PipelineStep::Plan)),
                (
                    "radon".to_string(),
                    HostEvent::Started(PipelineStep::Activate)
                ),
            ]
        );
        assert_eq!(step_events.get_events(), vec!["activation_start:radon"]);
    }

    #[test]
    fn test_noop_observer() {
        let observer = NoopObserver;
        observer.on_phase_start("p1", &[]);
        observer.on_activation_start("host1");
        observer.on_activation_complete("host1", true);
    }

    #[test]
    fn test_event_log_observer() {
        let log = EventLog::default();

        log.on_phase_start("p1", &["host1".to_string(), "host2".to_string()]);
        log.on_activation_start("host1");
        log.on_activation_complete("host1", true);

        let events = log.get_events();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0], "phase_start:p1:2");
        assert_eq!(events[1], "activation_start:host1");
        assert_eq!(events[2], "activation_complete:host1:ok");
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

        composite.on_phase_start("p1", &["host1".to_string()]);
        composite.on_activation_start("host1");

        // Both observers should have received events
        assert_eq!(log1.get_events().len(), 2);
        assert_eq!(log2.get_events().len(), 2);
    }
}
