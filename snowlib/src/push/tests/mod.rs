//! Tests for push deployment
//!
//! This module is split into focused test files:
//! - planning_tests: Tests for create_deployment_plans and reboot policies
//! - push_tests: Basic push() function tests
//! - workflow_tests: Full workflow integration tests
//! - profile_tests: Profile update tests
//! - hook_tests: Hook integration tests

mod hook_tests;
mod planning_tests;
mod profile_tests;
mod push_tests;
mod workflow_tests;

use super::planning::create_deployment_plans;
use super::*;
use crate::backends::deploy::DeployTarget;
use crate::backends::deploy::MockDeployBackend;
use crate::backends::nix::MockNixBackend;
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
    hook_backend: Arc<crate::backends::hook::MockHookBackend>,
    hosts: &[HostMetadata],
) -> crate::push::hooks::HookExecutor {
    let mut hooks_by_host = std::collections::HashMap::new();
    for host in hosts {
        hooks_by_host.insert(host.name.clone(), host.hooks.clone());
    }
    crate::push::hooks::HookExecutor::new(hook_backend, hooks_by_host)
}

/// Helper function to activate hosts (simplified for tests)
fn activate_plans_for_test(
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

        let result = deploy_backend
            .deploy(&plan.target, &plan.store_path, plan.activation_mode)
            .unwrap_or_else(|e| DeployResult::failure(e.to_string()));

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
}
