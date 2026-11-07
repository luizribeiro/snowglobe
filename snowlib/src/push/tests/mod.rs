//! Tests for push deployment
//!
//! This module is split into focused test files:
//! - planning_tests: Tests for create_deployment_plans and reboot policies
//! - push_tests: Basic push() function tests
//! - workflow_tests: Full workflow integration tests
//! - profile_tests: Profile update tests
//! - hook_tests: Hook integration tests
//! - host_event_tests: Per-host events reported to observers
//! - serial_group_tests: Hosts in a serial group deploying one at a time
//! - stage_order_tests: Order of planning, hooks, activation and reboot
//! - unchanged_tests: Skipping hosts that already run the built system

mod hook_tests;
mod host_event_tests;
mod planning_tests;
mod profile_tests;
mod push_tests;
mod serial_group_tests;
mod stage_order_tests;
mod unchanged_tests;
mod workflow_tests;

use super::planning::create_deployment_plans;
use super::*;
use crate::backends::deploy::DeployTarget;
use crate::backends::deploy::MockDeployBackend;
use crate::backends::nix::MockNixBackend;
use crate::hooks::HookStage;
use crate::host::HostMetadata;
use crate::nix::{ActivationMode, Flake, StorePath, SystemType};
use crate::observer::PushObserver;
use crate::reboot::RebootPolicy;
use std::sync::{Arc, Mutex};

fn create_push_config(strategy: PushStrategy, concurrency: usize) -> PushConfiguration {
    PushConfiguration {
        strategy,
        concurrency,
        output_mode: crate::process::OutputMode::Capture,
        ..PushConfiguration::default()
    }
}

/// Helper to create just HostMetadata for push tests
fn make_host_metadata(
    name: &str,
    phase: Option<&str>,
    reboot_policy: RebootPolicy,
) -> HostMetadata {
    HostMetadata {
        name: name.to_string(),
        phase: phase.map(|s| s.to_string()),
        tags: vec![],
        fqdn: format!("{}.example.com", name),
        reboot_policy,
        system_type: SystemType::NixOS,
        system: None,
        serial_group: None,
        ssh_port: None,
        optional: false,
        hooks: Vec::new(),
    }
}

/// Helper to create a tuple for create_deployment_plans tests
fn make_host(
    name: &str,
    phase: Option<&str>,
    reboot_policy: RebootPolicy,
) -> (HostMetadata, DeployTarget, StorePath) {
    let metadata = make_host_metadata(name, phase, reboot_policy);

    let target = DeployTarget {
        user: Some("root".to_string()),
        hostname: format!("{}.example.com", name),
        port: None,
    };

    let store_path = StorePath::new(format!("/nix/store/fake-{}", name));

    (metadata, target, store_path)
}

/// Helper to create a HookExecutor from a list of hosts
fn make_hook_executor(
    hook_backend: Arc<dyn crate::backends::hook::HookBackend>,
    hosts: &[HostMetadata],
) -> crate::push::hooks::HookExecutor {
    let mut hooks_by_host = std::collections::HashMap::new();
    for host in hosts {
        hooks_by_host.insert(host.name.clone(), host.hooks.clone());
    }
    crate::push::hooks::HookExecutor::new(hook_backend, hooks_by_host)
}

#[derive(Clone)]
struct TestObserver {
    events: Arc<Mutex<Vec<String>>>,
    done: Arc<Mutex<Vec<HostDeploymentResult>>>,
    completed: Arc<Mutex<Vec<PhaseResult>>>,
}

impl TestObserver {
    fn new() -> Self {
        Self {
            events: Arc::new(Mutex::new(Vec::new())),
            done: Arc::new(Mutex::new(Vec::new())),
            completed: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn get_events(&self) -> Vec<String> {
        self.events.lock().unwrap().clone()
    }

    fn record(&self, event: String) {
        self.events.lock().unwrap().push(event);
    }
}

/// The step-level names these tests have always asserted on, e.g.
/// "build_start:cerium" or "activation_complete:cerium:failed"
fn step_event_name(hostname: &str, event: &crate::observer::HostEvent) -> Option<String> {
    use crate::observer::HostEvent;
    let step_name = |step: &PipelineStep| match step {
        PipelineStep::Build => Some("build"),
        PipelineStep::Copy => Some("copy"),
        PipelineStep::Activate => Some("activation"),
        PipelineStep::Reboot => Some("reboot"),
        _ => None,
    };
    let failed = |step: &PipelineStep| {
        if *step == PipelineStep::Activate {
            "failed"
        } else {
            "fail"
        }
    };
    match event {
        HostEvent::Started(step) => Some(format!("{}_start:{hostname}", step_name(step)?)),
        HostEvent::Finished(step) => Some(format!("{}_complete:{hostname}:ok", step_name(step)?)),
        HostEvent::Failed { step, .. } => Some(format!(
            "{}_complete:{hostname}:{}",
            step_name(step)?,
            failed(step)
        )),
        HostEvent::Unchanged => Some(format!("unchanged:{hostname}")),
        HostEvent::WaitingForSerialGroup { group, holder } => Some(format!(
            "serial_wait:{hostname}:{group}:{}",
            holder.as_deref().unwrap_or("-")
        )),
        HostEvent::Done(_) => None,
    }
}

impl PushObserver for TestObserver {
    fn on_host_event(&self, hostname: &str, event: &crate::observer::HostEvent) {
        use crate::observer::HostEvent;
        match event {
            HostEvent::Started(step) => self.record(format!("started:{hostname}:{step}")),
            HostEvent::Finished(step) => self.record(format!("finished:{hostname}:{step}")),
            HostEvent::Failed { step, reason } => {
                self.record(format!("failed:{hostname}:{step}:{reason}"))
            }
            HostEvent::Done(result) => {
                self.done.lock().unwrap().push(result.clone());
                self.record(format!("done:{hostname}"));
            }
            HostEvent::Unchanged | HostEvent::WaitingForSerialGroup { .. } => {}
        }
        // Many tests assert on these step-level names as well
        if let Some(name) = step_event_name(hostname, event) {
            self.record(name);
        }
    }

    fn on_phase_start(&self, phase: &str, hosts: &[String]) {
        self.record(format!("phase_start:{}:{}", phase, hosts.len()));
    }

    fn on_phase_complete(&self, phase: &str, result: Option<&PhaseResult>) {
        let outcome = match result {
            Some(result) if result.all_succeeded() => "ok",
            Some(_) => "failed",
            None => "stopped",
        };
        self.record(format!("phase_complete:{phase}:{outcome}"));
        if let Some(result) = result {
            self.completed.lock().unwrap().push(result.clone());
        }
    }

    fn on_hooks_start(&self, hostname: &str, stage: HookStage) {
        self.record(format!("hooks_start:{}:{}", hostname, stage));
    }

    fn on_hooks_complete(&self, hostname: &str, stage: HookStage, success: bool) {
        self.record(format!(
            "hooks_complete:{}:{}:{}",
            hostname,
            stage,
            if success { "ok" } else { "fail" }
        ));
    }

    fn on_hook_start(&self, hostname: &str, stage: HookStage, hook_name: &str) {
        self.record(format!("hook_start:{}:{}:{}", hostname, stage, hook_name));
    }

    fn on_hook_complete(&self, hostname: &str, stage: HookStage, hook_name: &str, success: bool) {
        self.record(format!(
            "hook_complete:{}:{}:{}:{}",
            hostname,
            stage,
            hook_name,
            if success { "ok" } else { "fail" }
        ));
    }
}

/// Outcome of a push run through [`PushHarness`]
struct HarnessRun {
    results: PushResults,
    events: Vec<String>,
    /// Results reported through `HostEvent::Done`, in the order reported
    done: Vec<HostDeploymentResult>,
    completed: Vec<PhaseResult>,
    executed_hooks: Vec<(String, HookStage, crate::hooks::HookLocation, String)>,
}

/// Runs `push` with mock backends, hooks and a recording observer
struct PushHarness {
    nix_backend: MockNixBackend,
    deployer: MockDeployBackend,
    reboot_backend: crate::backends::reboot::MockRebootBackend,
    hook_backend: Arc<crate::backends::hook::MockHookBackend>,
    /// Runs hooks for real instead of through `hook_backend`
    real_hook_backend: Option<Arc<dyn crate::backends::hook::HookBackend>>,
    config: PushConfiguration,
}

impl PushHarness {
    fn new() -> Self {
        Self {
            nix_backend: MockNixBackend::new(),
            deployer: MockDeployBackend::new(),
            reboot_backend: crate::backends::reboot::MockRebootBackend::new(),
            hook_backend: Arc::new(crate::backends::hook::MockHookBackend::new()),
            real_hook_backend: None,
            config: PushConfiguration {
                output_mode: crate::process::OutputMode::Capture,
                ..PushConfiguration::default()
            },
        }
    }

    async fn run(&self, hosts: Vec<HostMetadata>) -> HarnessRun {
        let (results, observer) = self.run_push(hosts).await;
        let done = observer.done.lock().unwrap().clone();
        let completed = observer.completed.lock().unwrap().clone();
        HarnessRun {
            results: results.unwrap(),
            events: observer.get_events(),
            done,
            completed,
            executed_hooks: self.hook_backend.get_executed_hooks(),
        }
    }

    /// The push's outcome, including an error, and what the observer saw
    async fn run_push(&self, hosts: Vec<HostMetadata>) -> (Result<PushResults>, TestObserver) {
        let flake = Flake::test_context();
        let observer = TestObserver::new();
        let hook_backend = self
            .real_hook_backend
            .clone()
            .unwrap_or_else(|| self.hook_backend.clone());
        let hook_executor = make_hook_executor(hook_backend, &hosts);
        let push_ctx = PushContext::new(
            &flake,
            &self.nix_backend,
            &self.deployer,
            &self.reboot_backend,
            &self.config,
            &observer,
            Some(&hook_executor),
        );

        let results = push(&push_ctx, hosts).await;
        (results, observer)
    }
}

impl HarnessRun {
    fn host(&self, hostname: &str) -> &HostDeploymentResult {
        self.results
            .phases
            .iter()
            .flat_map(|phase| &phase.hosts)
            .find(|host| host.hostname == hostname)
            .unwrap_or_else(|| panic!("no result for {}", hostname))
    }

    fn has_event(&self, event: &str) -> bool {
        self.events.iter().any(|e| e == event)
    }

    fn has_event_starting_with(&self, prefix: &str) -> bool {
        self.events.iter().any(|e| e.starts_with(prefix))
    }
}

fn position(run: &HarnessRun, event: &str) -> usize {
    run.events
        .iter()
        .position(|e| e == event)
        .unwrap_or_else(|| panic!("missing {}: {:?}", event, run.events))
}

/// Asserts each host reported `Done` once, with the result push() returned,
/// and reported nothing after it
fn assert_done_with_final_results(run: &HarnessRun, hosts: &[&str]) {
    assert_eq!(run.done.len(), hosts.len(), "{:?}", run.done);
    for host in hosts {
        let reported: Vec<_> = run.done.iter().filter(|r| r.hostname == *host).collect();
        assert_eq!(reported, vec![run.host(host)], "{host}");
        let done = position(run, &format!("done:{host}"));
        let host_of = |event: &String| event.split(':').nth(1).map(str::to_string);
        assert!(
            run.events[done + 1..]
                .iter()
                .all(|e| host_of(e).as_deref() != Some(host)),
            "{host} reported after done: {:?}",
            run.events
        );
    }
}

/// The store path MockNixBackend builds for `hostname`
fn mock_built_system(hostname: &str) -> StorePath {
    StorePath::new(format!("/nix/store/mock-{}", hostname))
}

/// A host with a Kubernetes node's drain/uncordon hooks
fn kubernetes_node(name: &str) -> HostMetadata {
    let mut host = make_host_metadata(name, Some("p1"), RebootPolicy::Auto);
    host.hooks = kubernetes_node_hooks();
    host
}

/// One hook per stage, as a Kubernetes node would configure them
fn kubernetes_node_hooks() -> Vec<crate::hooks::HookConfig> {
    use crate::hooks::{HookConfig, HookLocation};
    vec![
        HookConfig::test_fixture(
            "drain-node",
            HookStage::PreDeploy,
            HookLocation::Local,
            "kubectl drain ${SNOW_HOSTNAME}",
        ),
        HookConfig::test_fixture(
            "uncordon-node",
            HookStage::PostDeploy,
            HookLocation::Local,
            "kubectl uncordon ${SNOW_HOSTNAME}",
        ),
        HookConfig::test_fixture(
            "wait-node-ready",
            HookStage::PostReboot,
            HookLocation::Local,
            "kubectl wait --for=condition=Ready node/${SNOW_HOSTNAME}",
        ),
        HookConfig::test_fixture(
            "recover-uncordon",
            HookStage::OnFailure,
            HookLocation::Local,
            "kubectl uncordon ${SNOW_HOSTNAME}",
        ),
    ]
}
