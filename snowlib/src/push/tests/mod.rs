//! Tests for push deployment
//!
//! This module is split into focused test files:
//! - planning_tests: Tests for create_deployment_plans and reboot policies
//! - push_tests: Basic push() function tests
//! - workflow_tests: Full workflow integration tests
//! - profile_tests: Profile update tests
//! - hook_tests: Hook integration tests
//! - stage_order_tests: Order of planning, hooks, activation and reboot
//! - unchanged_tests: Skipping hosts that already run the built system

mod hook_tests;
mod planning_tests;
mod profile_tests;
mod push_tests;
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
use crate::observer::{NoopObserver, PushObserver};
use crate::reboot::RebootPolicy;
use std::sync::{Arc, Mutex};

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
        serial_group: None,
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

/// Helper function to activate hosts (simplified for tests)
async fn activate_plans_for_test(
    deploy_backend: &dyn crate::backends::deploy::DeployBackend,
    plans: Vec<PushPlan>,
    _concurrency: usize,
    observer: &dyn crate::observer::PushObserver,
) -> Result<PushResults> {
    use crate::backends::deploy::DeployResult;

    let phase = plans
        .first()
        .and_then(|p| p.metadata.phase.as_ref())
        .cloned()
        .unwrap_or_else(|| "default".to_string());

    let hostnames: Vec<String> = plans.iter().map(|p| p.metadata.name.clone()).collect();

    observer.on_phase_start(&phase, &hostnames);

    let mut host_results = Vec::new();

    for plan in plans {
        let hostname = plan.metadata.name.clone();

        observer.on_activation_start(&hostname);

        let result = match deploy_backend
            .deploy(&plan.target, &plan.store_path, plan.activation_mode)
            .await
        {
            Ok(r) => r,
            Err(e) => DeployResult::failure(e.to_string()),
        };

        observer.on_activation_complete(&hostname, result.success);

        host_results.push(HostDeploymentResult {
            hostname,
            success: result.success,
            failed_at_step: if result.success {
                None
            } else {
                Some(PipelineStep::Activate)
            },
            activation_mode: Some(plan.activation_mode),
            rebooted: false,
            unchanged: false,
            error_message: if result.success {
                None
            } else {
                Some(result.message)
            },
        });
    }

    let phase_result = PhaseResult {
        phase: phase.clone(),
        hosts: host_results,
    };

    observer.on_phase_complete(&phase, phase_result.all_succeeded());

    Ok(PushResults {
        phases: vec![phase_result],
        skipped_phases: vec![],
    })
}

#[derive(Clone)]
struct TestObserver {
    events: Arc<Mutex<Vec<String>>>,
}

impl TestObserver {
    fn new() -> Self {
        Self {
            events: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn get_events(&self) -> Vec<String> {
        self.events.lock().unwrap().clone()
    }
}

impl PushObserver for TestObserver {
    fn on_build_step_start(&self, phase: &str, hosts: &[String]) {
        self.events
            .lock()
            .unwrap()
            .push(format!("build_step_start:{}:{}", phase, hosts.len()));
    }

    fn on_build_start(&self, hostname: &str) {
        self.events
            .lock()
            .unwrap()
            .push(format!("build_start:{}", hostname));
    }

    fn on_build_complete(&self, hostname: &str, success: bool) {
        self.events.lock().unwrap().push(format!(
            "build_complete:{}:{}",
            hostname,
            if success { "ok" } else { "fail" }
        ));
    }

    fn on_build_step_complete(&self, phase: &str, success: bool) {
        self.events.lock().unwrap().push(format!(
            "build_step_complete:{}:{}",
            phase,
            if success { "ok" } else { "fail" }
        ));
    }

    fn on_copy_step_start(&self, phase: &str, hosts: &[String]) {
        self.events
            .lock()
            .unwrap()
            .push(format!("copy_step_start:{}:{}", phase, hosts.len()));
    }

    fn on_copy_start(&self, hostname: &str) {
        self.events
            .lock()
            .unwrap()
            .push(format!("copy_start:{}", hostname));
    }

    fn on_copy_complete(&self, hostname: &str, success: bool) {
        self.events.lock().unwrap().push(format!(
            "copy_complete:{}:{}",
            hostname,
            if success { "ok" } else { "fail" }
        ));
    }

    fn on_copy_step_complete(&self, phase: &str, success: bool) {
        self.events.lock().unwrap().push(format!(
            "copy_step_complete:{}:{}",
            phase,
            if success { "ok" } else { "fail" }
        ));
    }

    fn on_phase_start(&self, phase: &str, hosts: &[String]) {
        self.events
            .lock()
            .unwrap()
            .push(format!("phase_start:{}:{}", phase, hosts.len()));
    }

    fn on_deploy_step_start(&self, phase: &str, hosts: &[String]) {
        self.events
            .lock()
            .unwrap()
            .push(format!("deploy_step_start:{}:{}", phase, hosts.len()));
    }

    fn on_activation_start(&self, hostname: &str) {
        self.events
            .lock()
            .unwrap()
            .push(format!("activation_start:{}", hostname));
    }

    fn on_host_unchanged(&self, hostname: &str) {
        self.events
            .lock()
            .unwrap()
            .push(format!("unchanged:{}", hostname));
    }

    fn on_activation_complete(&self, hostname: &str, success: bool) {
        self.events.lock().unwrap().push(format!(
            "activation_complete:{}:{}",
            hostname,
            if success { "ok" } else { "failed" }
        ));
    }

    fn on_deploy_step_complete(&self, phase: &str, success: bool) {
        self.events.lock().unwrap().push(format!(
            "deploy_step_complete:{}:{}",
            phase,
            if success { "ok" } else { "fail" }
        ));
    }

    fn on_phase_complete(&self, phase: &str, success: bool) {
        self.events.lock().unwrap().push(format!(
            "phase_complete:{}:{}",
            phase,
            if success { "ok" } else { "failed" }
        ));
    }

    fn on_reboot_start(&self, hostname: &str) {
        self.events
            .lock()
            .unwrap()
            .push(format!("reboot_start:{}", hostname));
    }

    fn on_reboot_complete(&self, hostname: &str, success: bool) {
        self.events.lock().unwrap().push(format!(
            "reboot_complete:{}:{}",
            hostname,
            if success { "ok" } else { "fail" }
        ));
    }

    fn on_reboot_step_start(&self, phase: &str, hosts: &[String]) {
        self.events
            .lock()
            .unwrap()
            .push(format!("reboot_step_start:{}:{}", phase, hosts.len()));
    }

    fn on_reboot_step_complete(&self, phase: &str, success: bool) {
        self.events.lock().unwrap().push(format!(
            "reboot_step_complete:{}:{}",
            phase,
            if success { "ok" } else { "fail" }
        ));
    }

    fn on_hooks_start(&self, hostname: &str, stage: HookStage) {
        self.events
            .lock()
            .unwrap()
            .push(format!("hooks_start:{}:{}", hostname, stage));
    }

    fn on_hooks_complete(&self, hostname: &str, stage: HookStage, success: bool) {
        self.events.lock().unwrap().push(format!(
            "hooks_complete:{}:{}:{}",
            hostname,
            stage,
            if success { "ok" } else { "fail" }
        ));
    }

    fn on_hook_start(&self, hostname: &str, stage: HookStage, hook_name: &str) {
        self.events
            .lock()
            .unwrap()
            .push(format!("hook_start:{}:{}:{}", hostname, stage, hook_name));
    }

    fn on_hook_complete(&self, hostname: &str, stage: HookStage, hook_name: &str, success: bool) {
        self.events.lock().unwrap().push(format!(
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

        let results = push(&push_ctx, hosts).await.unwrap();

        HarnessRun {
            results,
            events: observer.get_events(),
            executed_hooks: self.hook_backend.get_executed_hooks(),
        }
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

/// The store path MockNixBackend builds for `hostname`
fn mock_built_system(hostname: &str) -> StorePath {
    StorePath::new(format!("/nix/store/mock-{}", hostname))
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
