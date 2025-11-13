//! Push deployment orchestration
//!
//! This module provides the main `push()` function for deploying NixOS configurations
//! to remote hosts. The deployment process is organized as a pipeline of steps:
//!
//! 1. **Build**: Builds NixOS configurations for each host
//! 2. **Copy**: Copies closures to remote hosts
//! 3. **Planning**: Creates deployment plans with reboot detection
//! 4. **Activate**: Activates configurations on remote hosts
//! 5. **Reboot**: Reboots hosts that require it
//!
//! Each step is implemented as a separate module under `steps/` and implements
//! the `Step` trait defined in `step.rs`.

mod config;
mod context;
mod results;
mod step;
pub mod steps;

// Re-export public types
pub use config::{DeploymentMode, PushConfiguration};
pub use context::PushContext;
pub use results::{BlockedHost, PushPlan, PushResults};
pub use step::{Step, StepResult};
pub use steps::{
    ActivateInput, ActivateOutput, ActivateStep, BuildInput, BuildOutput, BuildStep, CopyInput,
    CopyOutput, CopyStep, PlanningInput, PlanningOutput, PlanningStep, RebootInput, RebootOutput,
    RebootStep,
};

// Re-export create_deployment_plans for backward compatibility and testing
pub use steps::plan::create_deployment_plans;

use crate::error::Result;
use crate::nix::{Flake, HostMetadata};
use crate::observer::PushObserver;
use std::collections::HashMap;

/// Main push orchestration function - handles complete deployment workflow
///
/// This is the primary entry point for deploying configurations. It orchestrates
/// the entire deployment process by phase:
///
/// For each phase (sequential):
/// 1. **Build** - Build NixOS configurations for all hosts in the phase
/// 2. **Copy** - Copy closures to remote hosts
/// 3. **Plan** - Create deployment plans with reboot detection
/// 4. **Activate** - Activate configurations (parallel within phase)
/// 5. **Reboot** - Reboot hosts that need it
///
/// If any step fails in a phase, deployment stops immediately.
///
/// # Arguments
///
/// - `ctx`: Nix context (flake root)
/// - `nix_backend`: Nix backend implementation (for building)
/// - `deploy_backend`: Deployment backend implementation (for copying and activation)
/// - `reboot_backend`: Reboot backend implementation (detection and execution)
/// - `hosts`: List of hosts to deploy
/// - `config`: Push configuration (mode, concurrency, etc.)
/// - `observer`: Observer to receive lifecycle events
///
/// # Example
///
/// ```no_run
/// use snowlib::{push, PushConfiguration, PushContext, Flake, SystemNixBackend, SshDeployBackend, SystemRebootBackend};
/// use snowlib::observer::NoopObserver;
/// use snowlib::nix::HostMetadata;
///
/// # fn example(hosts: Vec<HostMetadata>) -> snowlib::Result<()> {
/// let flake = Flake::new(None)?;
/// let nix_backend = SystemNixBackend::new();
/// let deploy_backend = SshDeployBackend::new();
/// let reboot_backend = SystemRebootBackend::default();
/// let config = PushConfiguration::default();
/// let push_ctx = PushContext::new(&nix_backend, &deploy_backend, &reboot_backend, &config);
/// let observer = NoopObserver;
///
/// let results = push(&flake, &push_ctx, hosts, &observer)?;
/// # Ok(())
/// # }
/// ```
pub fn push(
    flake: &Flake,
    push_ctx: &PushContext,
    hosts: Vec<HostMetadata>,
    observer: &dyn PushObserver,
) -> Result<PushResults> {
    // Group hosts by phase
    let by_phase = group_by_phase(hosts, push_ctx.config.ignore_phases);
    let phases = sorted_phases(&by_phase);

    let mut all_results = Vec::new();

    // Execute each phase sequentially
    for phase in phases {
        let phase_hosts = by_phase.get(&phase).unwrap();

        // Collect hostnames for phase lifecycle notification
        let hostnames: Vec<String> = phase_hosts.iter().map(|h| h.name.clone()).collect();

        // Notify observer of phase start
        observer.on_phase_start(&phase, &hostnames)?;

        // Execute phase pipeline
        let phase_result = execute_phase(&phase, phase_hosts, flake, push_ctx, observer);

        // Notify observer of phase completion (always, even on failure)
        let phase_success = phase_result
            .as_ref()
            .map(|results| results.iter().all(|(_, r)| r.success))
            .unwrap_or(false);
        observer.on_phase_complete(&phase, phase_success)?;

        // Handle phase result
        match phase_result {
            Ok(phase_results) => {
                all_results.extend(phase_results);

                // If this phase failed, stop deployment
                if !phase_success {
                    return Ok(PushResults {
                        results: all_results,
                    });
                }
            }
            Err(e) => {
                // Phase failed with error - stop deployment
                return Err(e);
            }
        }
    }

    Ok(PushResults {
        results: all_results,
    })
}

/// Execute all deployment steps for a single phase
///
/// This function runs the complete deployment pipeline for hosts in a phase:
/// 1. Build - Compile NixOS configurations
/// 2. Copy - Transfer closures to remote hosts
/// 3. Plan - Create deployment plans with reboot detection
/// 4. Activate - Activate new configurations
/// 5. Reboot - Reboot hosts if needed
///
/// Returns deployment results for all hosts in the phase.
fn execute_phase(
    phase: &str,
    phase_hosts: &[HostMetadata],
    flake: &Flake,
    push_ctx: &PushContext,
    observer: &dyn PushObserver,
) -> Result<Vec<(String, crate::backends::deploy::DeployResult)>> {
    use tracing::info_span;

    // Create phase span
    let phase_span = info_span!("phase", phase = phase, hosts = phase_hosts.len());
    let _entered = phase_span.enter();

    // Step 1: Build
    let build_inputs: Vec<BuildInput> = phase_hosts
        .iter()
        .map(|h| BuildInput {
            metadata: h.clone(),
        })
        .collect();
    let build_step = BuildStep { flake, push_ctx };
    let build_outputs = execute_step(&build_step, phase, build_inputs, observer)?;

    // Step 2: Copy
    let copy_step = CopyStep { push_ctx };
    let copy_outputs = execute_step(&copy_step, phase, build_outputs, observer)?;

    // Step 3: Planning
    let plan_step = PlanningStep { push_ctx };
    let planning_outputs = execute_step(&plan_step, phase, copy_outputs, observer)?;

    // Step 4: Activate
    let activate_inputs: Vec<ActivateInput> = planning_outputs
        .into_iter()
        .map(|p| ActivateInput { plan: p.plan })
        .collect();
    let activate_step = ActivateStep { push_ctx };
    let activate_outputs = execute_step(&activate_step, phase, activate_inputs, observer)?;

    // Step 5: Reboot
    let reboot_inputs: Vec<RebootInput> = activate_outputs
        .into_iter()
        .map(|a| RebootInput {
            plan: a.plan,
            deploy_result: a.result,
        })
        .collect();
    let reboot_step = RebootStep { push_ctx };
    let reboot_outputs = execute_step(&reboot_step, phase, reboot_inputs, observer)?;

    // Collect results from reboot step
    let phase_results: Vec<(String, crate::backends::deploy::DeployResult)> = reboot_outputs
        .into_iter()
        .map(|r| (r.hostname, r.result))
        .collect();

    Ok(phase_results)
}

/// Helper function to execute a step and handle failures
fn execute_step<S: Step>(
    step: &S,
    phase: &str,
    inputs: Vec<S::Input>,
    observer: &dyn PushObserver,
) -> Result<Vec<S::Output>> {
    let results = step.execute_batch(phase, inputs, observer)?;

    // Check for failures and collect successes
    let mut outputs = Vec::new();
    for result in results {
        match result {
            StepResult::Success(output) => outputs.push(output),
            StepResult::Failed { error, .. } => {
                // Early exit on first failure
                return Err(error);
            }
        }
    }
    Ok(outputs)
}

/// Group hosts by phase
fn group_by_phase(
    hosts: Vec<HostMetadata>,
    ignore_phases: bool,
) -> HashMap<String, Vec<HostMetadata>> {
    let mut by_phase: HashMap<String, Vec<HostMetadata>> = HashMap::new();

    for host in hosts {
        let phase = if ignore_phases {
            "_all".to_string()
        } else {
            host.phase.clone().unwrap_or_else(|| "default".to_string())
        };
        by_phase.entry(phase).or_default().push(host);
    }

    by_phase
}

/// Get sorted list of phases
fn sorted_phases(by_phase: &HashMap<String, Vec<HostMetadata>>) -> Vec<String> {
    let mut phases: Vec<String> = by_phase.keys().cloned().collect();
    phases.sort();
    phases
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backends::deploy::{ActivationMode, DeployResult, DeployTarget};
    use crate::backends::mock::{MockDeployBackend, MockNixBackend};
    use crate::nix::{Flake, HostMetadata, StorePath};

    /// Helper to create just HostMetadata for push tests
    fn make_host_metadata(
        name: &str,
        phase: Option<&str>,
        reboot_policy: crate::nix::RebootPolicy,
    ) -> HostMetadata {
        HostMetadata {
            name: name.to_string(),
            phase: phase.map(|s| s.to_string()),
            tags: vec![],
            fqdn: format!("{}.example.com", name),
            reboot_policy,
        }
    }

    /// Helper to create a tuple for create_deployment_plans tests
    fn make_host(
        name: &str,
        phase: Option<&str>,
        reboot_policy: crate::nix::RebootPolicy,
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

    /// Helper function to activate hosts using ActivateStep (replaces old activate_plans_for_test)
    fn activate_plans_for_test(
        deploy_backend: &dyn crate::backends::deploy::DeployBackend,
        plans: Vec<PushPlan>,
        concurrency: usize,
        observer: &dyn crate::observer::PushObserver,
    ) -> Result<PushResults> {
        // Get the phase from the first plan (all plans in tests have the same phase)
        let phase = plans
            .first()
            .and_then(|p| p.metadata.phase.as_ref())
            .cloned()
            .unwrap_or_else(|| "default".to_string());

        // Collect hostnames for phase lifecycle notification
        let hostnames: Vec<String> = plans.iter().map(|p| p.metadata.name.clone()).collect();

        // Notify observer of phase start
        observer.on_phase_start(&phase, &hostnames)?;

        // Convert plans to ActivateInput
        let inputs: Vec<ActivateInput> = plans
            .into_iter()
            .map(|plan| ActivateInput { plan })
            .collect();

        // Create minimal backends and config for PushContext (only concurrency is used)
        let nix_backend = MockNixBackend::new();
        let reboot_backend = SystemRebootBackend::default();
        let config = PushConfiguration {
            concurrency,
            ..Default::default()
        };
        let push_ctx = PushContext::new(&nix_backend, deploy_backend, &reboot_backend, &config);

        // Create ActivateStep and execute
        let step = ActivateStep {
            push_ctx: &push_ctx,
        };

        // Execute and handle potential error (ActivateStep returns error on phase failure)
        let results_or_err = step.execute_batch(&phase, inputs, observer);

        // Convert StepResults to PushResults format
        // Even if there's an error, we might have partial results we want to return
        let mut push_results = Vec::new();
        let phase_success;

        match results_or_err {
            Ok(results) => {
                phase_success = results.iter().all(|r| r.is_success());
                for result in results {
                    match result {
                        StepResult::Success(output) => {
                            push_results.push((output.hostname, output.result));
                        }
                        StepResult::Failed { hostname, error } => {
                            // In case of failure, create a DeployResult with error
                            push_results.push((hostname, DeployResult::failure(error.to_string())));
                        }
                    }
                }
            }
            Err(e) => {
                phase_success = false;
                // Notify observer of phase completion with failure
                observer.on_phase_complete(&phase, phase_success)?;
                // On error, return the error - tests that expect failures will handle this
                return Err(e);
            }
        }

        // Notify observer of phase completion
        observer.on_phase_complete(&phase, phase_success)?;

        Ok(PushResults {
            results: push_results,
        })
    }

    #[test]
    fn test_push_results_counts() {
        let results = PushResults {
            results: vec![
                ("host1".to_string(), DeployResult::success("ok")),
                ("host2".to_string(), DeployResult::failure("failed")),
                ("host3".to_string(), DeployResult::success("ok")),
            ],
        };

        assert_eq!(results.success_count(), 2);
        assert_eq!(results.failure_count(), 1);
        assert!(!results.all_succeeded());
    }

    #[test]
    fn test_push_results_all_succeeded() {
        let results = PushResults {
            results: vec![
                ("host1".to_string(), DeployResult::success("ok")),
                ("host2".to_string(), DeployResult::success("ok")),
            ],
        };

        assert_eq!(results.success_count(), 2);
        assert_eq!(results.failure_count(), 0);
        assert!(results.all_succeeded());
    }

    // Tests for create_deployment_plans
    use crate::backends::reboot::{RebootDetectionConfig, SystemRebootBackend};

    #[test]
    fn test_create_deployment_plans_auto_mode_no_reboot() {
        // Create a detector that will never detect reboot needed
        // This simulates the edge case where:
        // - No kernel/initrd/init changes
        // - No critical services to restart (empty list)
        // - /run/nixos/dry-activation-restart-list won't exist on remote
        // This configuration was causing "No such file or directory" errors
        // before we added the file existence check in check_critical_services()
        let config = RebootDetectionConfig {
            critical_services: vec![],
            check_kernel: false,
            check_initrd: false,
            check_init: false,
        };
        let detector = SystemRebootBackend::new(config);

        let hosts = vec![make_host("test", None, crate::nix::RebootPolicy::Auto)];

        let (plans, blocked) =
            create_deployment_plans(hosts, &detector, DeploymentMode::Auto, false).unwrap();

        assert_eq!(plans.len(), 1);
        assert_eq!(blocked.len(), 0);
        // Should use Switch mode since no reboot is required
        assert_eq!(plans[0].activation_mode, ActivationMode::Switch);
        // Reboot check should indicate no reboot needed
        assert!(plans[0].reboot_check.is_some());
        assert!(!plans[0].reboot_check.as_ref().unwrap().required);
    }

    #[test]
    fn test_create_deployment_plans_dry_run() {
        let detector = SystemRebootBackend::default();
        let hosts = vec![make_host("test", None, crate::nix::RebootPolicy::Auto)];

        let (plans, blocked) =
            create_deployment_plans(hosts, &detector, DeploymentMode::DryRun, false).unwrap();

        assert_eq!(plans.len(), 1);
        assert_eq!(blocked.len(), 0);
        assert_eq!(plans[0].activation_mode, ActivationMode::DryActivate);
        assert!(plans[0].reboot_check.is_none());
    }

    #[test]
    fn test_create_deployment_plans_boot_mode() {
        let detector = SystemRebootBackend::default();
        let hosts = vec![make_host("test", None, crate::nix::RebootPolicy::Auto)];

        let (plans, blocked) =
            create_deployment_plans(hosts, &detector, DeploymentMode::Boot, false).unwrap();

        assert_eq!(plans.len(), 1);
        assert_eq!(blocked.len(), 0);
        assert_eq!(plans[0].activation_mode, ActivationMode::Boot);
        assert!(plans[0].reboot_check.is_none());
    }

    #[test]
    fn test_create_deployment_plans_test_mode() {
        let detector = SystemRebootBackend::default();
        let hosts = vec![make_host("test", None, crate::nix::RebootPolicy::Auto)];

        let (plans, blocked) =
            create_deployment_plans(hosts, &detector, DeploymentMode::Test, false).unwrap();

        assert_eq!(plans.len(), 1);
        assert_eq!(blocked.len(), 0);
        assert_eq!(plans[0].activation_mode, ActivationMode::Test);
        assert!(plans[0].reboot_check.is_none());
    }

    #[test]
    fn test_create_deployment_plans_switch_with_force() {
        // In reality, reboot would be detected, but with force flag it should still use Switch
        let detector = SystemRebootBackend::default();
        let hosts = vec![make_host("test", None, crate::nix::RebootPolicy::Auto)];

        let (plans, blocked) =
            create_deployment_plans(hosts, &detector, DeploymentMode::Switch, true).unwrap();

        assert_eq!(plans.len(), 1);
        assert_eq!(blocked.len(), 0);
        // Force flag causes switch mode even if reboot detected
        assert_eq!(plans[0].activation_mode, ActivationMode::Switch);
    }

    #[test]
    fn test_create_deployment_plans_multiple_hosts_mixed_results() {
        // Create a detector that will never detect reboot needed
        let config = RebootDetectionConfig {
            critical_services: vec![],
            check_kernel: false,
            check_initrd: false,
            check_init: false,
        };
        let detector = SystemRebootBackend::new(config);

        let hosts = vec![
            make_host("host1", None, crate::nix::RebootPolicy::Auto),
            make_host("host2", None, crate::nix::RebootPolicy::Auto),
            make_host("host3", None, crate::nix::RebootPolicy::Auto),
        ];

        let (plans, blocked) =
            create_deployment_plans(hosts, &detector, DeploymentMode::Auto, false).unwrap();

        assert_eq!(plans.len(), 3);
        assert_eq!(blocked.len(), 0);
        // All should use switch mode since no reboot detected
        for plan in plans {
            assert_eq!(plan.activation_mode, ActivationMode::Switch);
        }
    }

    #[test]
    fn test_reboot_policy_never_uses_boot_mode() {
        let config = RebootDetectionConfig {
            critical_services: vec![],
            check_kernel: false,
            check_initrd: false,
            check_init: false,
        };
        let detector = SystemRebootBackend::new(config);

        let hosts = vec![make_host("host1", None, crate::nix::RebootPolicy::Never)];

        let (plans, blocked) =
            create_deployment_plans(hosts, &detector, DeploymentMode::Auto, false).unwrap();

        assert_eq!(plans.len(), 1);
        assert_eq!(blocked.len(), 0);
        // Should use boot mode due to 'never' policy
        assert_eq!(plans[0].activation_mode, ActivationMode::Boot);
        // Should indicate no reboot required
        assert!(plans[0].reboot_check.is_some());
        assert!(!plans[0].reboot_check.as_ref().unwrap().required);
    }

    #[test]
    fn test_reboot_policy_always_uses_boot_mode_with_reboot() {
        let config = RebootDetectionConfig {
            critical_services: vec![],
            check_kernel: false,
            check_initrd: false,
            check_init: false,
        };
        let detector = SystemRebootBackend::new(config);

        let hosts = vec![make_host("host1", None, crate::nix::RebootPolicy::Always)];

        let (plans, blocked) =
            create_deployment_plans(hosts, &detector, DeploymentMode::Auto, false).unwrap();

        assert_eq!(plans.len(), 1);
        assert_eq!(blocked.len(), 0);
        // Should use boot mode due to 'always' policy
        assert_eq!(plans[0].activation_mode, ActivationMode::Boot);
        // Should indicate reboot required
        assert!(plans[0].reboot_check.is_some());
        assert!(plans[0].reboot_check.as_ref().unwrap().required);
    }

    #[test]
    fn test_reboot_policy_auto_uses_detection() {
        let config = RebootDetectionConfig {
            critical_services: vec![],
            check_kernel: false,
            check_initrd: false,
            check_init: false,
        };
        let detector = SystemRebootBackend::new(config);

        let hosts = vec![make_host("host1", None, crate::nix::RebootPolicy::Auto)];

        let (plans, blocked) =
            create_deployment_plans(hosts, &detector, DeploymentMode::Auto, false).unwrap();

        assert_eq!(plans.len(), 1);
        assert_eq!(blocked.len(), 0);
        // Should use switch mode (no reboot detected)
        assert_eq!(plans[0].activation_mode, ActivationMode::Switch);
        assert!(plans[0].reboot_check.is_some());
        assert!(!plans[0].reboot_check.as_ref().unwrap().required);
    }

    #[test]
    fn test_reboot_policy_never_overrides_switch_mode() {
        let config = RebootDetectionConfig {
            critical_services: vec![],
            check_kernel: false,
            check_initrd: false,
            check_init: false,
        };
        let detector = SystemRebootBackend::new(config);

        let hosts = vec![make_host("host1", None, crate::nix::RebootPolicy::Never)];

        let (plans, blocked) =
            create_deployment_plans(hosts, &detector, DeploymentMode::Switch, false).unwrap();

        assert_eq!(plans.len(), 1);
        assert_eq!(blocked.len(), 0);
        // Should use boot mode despite switch flag (policy overrides)
        assert_eq!(plans[0].activation_mode, ActivationMode::Boot);
        assert!(!plans[0].reboot_check.as_ref().unwrap().required);
    }

    #[test]
    fn test_reboot_policy_always_overrides_switch_mode() {
        let config = RebootDetectionConfig {
            critical_services: vec![],
            check_kernel: false,
            check_initrd: false,
            check_init: false,
        };
        let detector = SystemRebootBackend::new(config);

        let hosts = vec![make_host("host1", None, crate::nix::RebootPolicy::Always)];

        let (plans, blocked) =
            create_deployment_plans(hosts, &detector, DeploymentMode::Switch, false).unwrap();

        assert_eq!(plans.len(), 1);
        assert_eq!(blocked.len(), 0);
        // Should use boot mode despite switch flag (policy overrides)
        assert_eq!(plans[0].activation_mode, ActivationMode::Boot);
        assert!(plans[0].reboot_check.as_ref().unwrap().required);
    }

    // Tests for the new push() function with observer support
    use crate::observer::{NoopObserver, PushObserver};
    use std::sync::{Arc, Mutex};

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
        fn on_build_step_start(&self, phase: &str, hosts: &[String]) -> crate::error::Result<()> {
            self.events
                .lock()
                .unwrap()
                .push(format!("build_step_start:{}:{}", phase, hosts.len()));
            Ok(())
        }

        fn on_build_start(&self, hostname: &str) -> crate::error::Result<()> {
            self.events
                .lock()
                .unwrap()
                .push(format!("build_start:{}", hostname));
            Ok(())
        }

        fn on_build_complete(&self, hostname: &str, success: bool) -> crate::error::Result<()> {
            self.events.lock().unwrap().push(format!(
                "build_complete:{}:{}",
                hostname,
                if success { "ok" } else { "fail" }
            ));
            Ok(())
        }

        fn on_build_step_complete(&self, phase: &str, success: bool) -> crate::error::Result<()> {
            self.events.lock().unwrap().push(format!(
                "build_step_complete:{}:{}",
                phase,
                if success { "ok" } else { "fail" }
            ));
            Ok(())
        }

        fn on_copy_step_start(&self, phase: &str, hosts: &[String]) -> crate::error::Result<()> {
            self.events
                .lock()
                .unwrap()
                .push(format!("copy_step_start:{}:{}", phase, hosts.len()));
            Ok(())
        }

        fn on_copy_start(&self, hostname: &str) -> crate::error::Result<()> {
            self.events
                .lock()
                .unwrap()
                .push(format!("copy_start:{}", hostname));
            Ok(())
        }

        fn on_copy_complete(&self, hostname: &str, success: bool) -> crate::error::Result<()> {
            self.events.lock().unwrap().push(format!(
                "copy_complete:{}:{}",
                hostname,
                if success { "ok" } else { "fail" }
            ));
            Ok(())
        }

        fn on_copy_step_complete(&self, phase: &str, success: bool) -> crate::error::Result<()> {
            self.events.lock().unwrap().push(format!(
                "copy_step_complete:{}:{}",
                phase,
                if success { "ok" } else { "fail" }
            ));
            Ok(())
        }

        fn on_phase_start(&self, phase: &str, hosts: &[String]) -> crate::error::Result<()> {
            self.events
                .lock()
                .unwrap()
                .push(format!("phase_start:{}:{}", phase, hosts.len()));
            Ok(())
        }

        fn on_deploy_step_start(&self, phase: &str, hosts: &[String]) -> crate::error::Result<()> {
            self.events.lock().unwrap().push(format!(
                "deploy_step_start:{}:{}",
                phase,
                hosts.len()
            ));
            Ok(())
        }

        fn on_deploy_start(
            &self,
            hostname: &str,
            _mode: ActivationMode,
        ) -> crate::error::Result<()> {
            self.events
                .lock()
                .unwrap()
                .push(format!("deploy_start:{}", hostname));
            Ok(())
        }

        fn on_deploy_complete(
            &self,
            hostname: &str,
            result: &DeployResult,
        ) -> crate::error::Result<()> {
            self.events.lock().unwrap().push(format!(
                "deploy_complete:{}:{}",
                hostname,
                if result.success { "ok" } else { "failed" }
            ));
            Ok(())
        }

        fn on_deploy_step_complete(&self, phase: &str, success: bool) -> crate::error::Result<()> {
            self.events.lock().unwrap().push(format!(
                "deploy_step_complete:{}:{}",
                phase,
                if success { "ok" } else { "fail" }
            ));
            Ok(())
        }

        fn on_phase_complete(&self, phase: &str, success: bool) -> crate::error::Result<()> {
            self.events.lock().unwrap().push(format!(
                "phase_complete:{}:{}",
                phase,
                if success { "ok" } else { "failed" }
            ));
            Ok(())
        }

        fn on_reboot_start(&self, hostname: &str) -> crate::error::Result<()> {
            self.events
                .lock()
                .unwrap()
                .push(format!("reboot_start:{}", hostname));
            Ok(())
        }

        fn on_reboot_complete(&self, hostname: &str, success: bool) -> crate::error::Result<()> {
            self.events.lock().unwrap().push(format!(
                "reboot_complete:{}:{}",
                hostname,
                if success { "ok" } else { "fail" }
            ));
            Ok(())
        }

        fn on_reboot_step_start(&self, phase: &str, hosts: &[String]) -> crate::error::Result<()> {
            self.events.lock().unwrap().push(format!(
                "reboot_step_start:{}:{}",
                phase,
                hosts.len()
            ));
            Ok(())
        }

        fn on_reboot_step_complete(&self, phase: &str, success: bool) -> crate::error::Result<()> {
            self.events.lock().unwrap().push(format!(
                "reboot_step_complete:{}:{}",
                phase,
                if success { "ok" } else { "fail" }
            ));
            Ok(())
        }
    }

    #[test]
    fn test_push_with_single_phase() {
        let deployer = MockDeployBackend::new();
        let observer = TestObserver::new();

        let plans = vec![
            PushPlan {
                metadata: HostMetadata {
                    name: "host1".to_string(),
                    phase: Some("p1".to_string()),
                    tags: vec![],
                    fqdn: "host1.example.com".to_string(),
                    reboot_policy: crate::nix::RebootPolicy::Auto,
                },
                target: DeployTarget {
                    user: Some("root".to_string()),
                    hostname: "host1.example.com".to_string(),
                    port: None,
                },
                store_path: StorePath::new("/nix/store/fake-host1".to_string()),
                activation_mode: ActivationMode::Switch,
                reboot_check: None,
            },
            PushPlan {
                metadata: HostMetadata {
                    name: "host2".to_string(),
                    phase: Some("p1".to_string()),
                    tags: vec![],
                    fqdn: "host2.example.com".to_string(),
                    reboot_policy: crate::nix::RebootPolicy::Auto,
                },
                target: DeployTarget {
                    user: Some("root".to_string()),
                    hostname: "host2.example.com".to_string(),
                    port: None,
                },
                store_path: StorePath::new("/nix/store/fake-host2".to_string()),
                activation_mode: ActivationMode::Switch,
                reboot_check: None,
            },
        ];

        let results = activate_plans_for_test(&deployer, plans, 2, &observer).unwrap();

        assert_eq!(results.success_count(), 2);
        assert_eq!(results.failure_count(), 0);

        // Check observer events
        let events = observer.get_events();
        assert!(events.contains(&"phase_start:p1:2".to_string()));
        assert!(events.contains(&"deploy_start:host1".to_string()));
        assert!(events.contains(&"deploy_start:host2".to_string()));
        assert!(events.contains(&"deploy_complete:host1:ok".to_string()));
        assert!(events.contains(&"deploy_complete:host2:ok".to_string()));
        assert!(events.contains(&"phase_complete:p1:ok".to_string()));
    }

    #[test]
    fn test_push_with_multiple_phases() {
        // Test that phases execute in lexicographic order (p1 before p2)
        let flake = Flake::test_context();
        let nix_backend = MockNixBackend::new();
        let deployer = MockDeployBackend::new();
        let reboot_config = crate::backends::reboot::RebootDetectionConfig {
            critical_services: vec![],
            check_kernel: false,
            check_initrd: false,
            check_init: false,
        };
        let reboot_detector = crate::backends::reboot::SystemRebootBackend::new(reboot_config);
        let observer = TestObserver::new();

        // Note: hosts are intentionally out of order to test sorting
        let hosts = vec![
            HostMetadata {
                name: "host3".to_string(),
                fqdn: "host3.example.com".to_string(),
                phase: Some("p2".to_string()),
                tags: vec![],
                reboot_policy: crate::nix::RebootPolicy::Auto,
            },
            HostMetadata {
                name: "host1".to_string(),
                fqdn: "host1.example.com".to_string(),
                phase: Some("p1".to_string()),
                tags: vec![],
                reboot_policy: crate::nix::RebootPolicy::Auto,
            },
        ];

        let config = PushConfiguration {
            mode: DeploymentMode::Switch,
            force: false,
            concurrency: 2,
            ignore_phases: false,
            quiet: false,
            ssh_user: None,
            allow_reboot: true,
        };

        let results = {
            let push_ctx = PushContext::new(&nix_backend, &deployer, &reboot_detector, &config);
            push(&flake, &push_ctx, hosts, &observer)
        }
        .unwrap();

        assert_eq!(results.success_count(), 2);

        // Check that phases were executed in order
        let events = observer.get_events();
        let p1_start = events.iter().position(|e| e == "phase_start:p1:1").unwrap();
        let p2_start = events.iter().position(|e| e == "phase_start:p2:1").unwrap();

        // p1 should start before p2 (lexicographic order)
        assert!(p1_start < p2_start);
    }

    #[test]
    fn test_push_stops_on_phase_failure() {
        // Test that when phase p1 fails, phase p2 never starts
        let flake = Flake::test_context();
        let nix_backend = MockNixBackend::new();
        let deployer =
            MockDeployBackend::new().with_activate_failures(vec!["host1.example.com".to_string()]);
        let reboot_config = crate::backends::reboot::RebootDetectionConfig {
            critical_services: vec![],
            check_kernel: false,
            check_initrd: false,
            check_init: false,
        };
        let reboot_detector = crate::backends::reboot::SystemRebootBackend::new(reboot_config);
        let observer = TestObserver::new();

        let hosts = vec![
            HostMetadata {
                name: "host1".to_string(),
                fqdn: "host1.example.com".to_string(),
                phase: Some("p1".to_string()),
                tags: vec![],
                reboot_policy: crate::nix::RebootPolicy::Auto,
            },
            HostMetadata {
                name: "host2".to_string(),
                fqdn: "host2.example.com".to_string(),
                phase: Some("p2".to_string()),
                tags: vec![],
                reboot_policy: crate::nix::RebootPolicy::Auto,
            },
        ];

        let config = PushConfiguration {
            mode: DeploymentMode::Switch,
            force: false,
            concurrency: 2,
            ignore_phases: false,
            quiet: false,
            ssh_user: None,
            allow_reboot: true,
        };

        // When activation fails, push() returns Err
        let result = {
            let push_ctx = PushContext::new(&nix_backend, &deployer, &reboot_detector, &config);
            push(&flake, &push_ctx, hosts, &observer)
        };

        // Push should fail due to phase p1 activation failure
        assert!(
            result.is_err(),
            "Push should return error when activation fails"
        );

        let events = observer.get_events();

        // Phase p1 should have completed (with failure)
        assert!(events.contains(&"phase_complete:p1:failed".to_string()));

        // Phase p2 should never start (no build, copy, or deploy events for host2)
        assert!(
            !events.iter().any(|e| e.contains("host2")),
            "Phase p2 should not have started after p1 failure"
        );
    }

    #[test]
    fn test_push_with_noop_observer() {
        let deployer = MockDeployBackend::new();
        let observer = NoopObserver;

        let plans = vec![PushPlan {
            metadata: HostMetadata {
                name: "host1".to_string(),
                phase: Some("p1".to_string()),
                tags: vec![],
                fqdn: "host1.example.com".to_string(),
                reboot_policy: crate::nix::RebootPolicy::Auto,
            },
            target: DeployTarget {
                user: Some("root".to_string()),
                hostname: "host1.example.com".to_string(),
                port: None,
            },
            store_path: StorePath::new("/nix/store/fake-host1".to_string()),
            activation_mode: ActivationMode::Switch,
            reboot_check: None,
        }];

        let results = activate_plans_for_test(&deployer, plans, 1, &observer).unwrap();
        assert_eq!(results.success_count(), 1);
    }

    #[test]
    fn test_push_with_default_phase() {
        let deployer = MockDeployBackend::new();
        let observer = TestObserver::new();

        let plans = vec![PushPlan {
            metadata: HostMetadata {
                name: "host1".to_string(),
                phase: None, // No phase specified
                tags: vec![],
                fqdn: "host1.example.com".to_string(),
                reboot_policy: crate::nix::RebootPolicy::Auto,
            },
            target: DeployTarget {
                user: Some("root".to_string()),
                hostname: "host1.example.com".to_string(),
                port: None,
            },
            store_path: StorePath::new("/nix/store/fake-host1".to_string()),
            activation_mode: ActivationMode::Switch,
            reboot_check: None,
        }];

        let results = activate_plans_for_test(&deployer, plans, 1, &observer).unwrap();
        assert_eq!(results.success_count(), 1);

        // Should use "default" phase
        let events = observer.get_events();
        assert!(events.contains(&"phase_start:default:1".to_string()));
    }

    #[test]
    fn test_push_with_partial_failure_in_phase() {
        // Test that when one host fails in a phase, we still get results for all hosts in that phase
        let deployer =
            MockDeployBackend::new().with_failures(vec!["host2.example.com".to_string()]);
        let observer = TestObserver::new();

        let plans = vec![
            PushPlan {
                metadata: HostMetadata {
                    name: "host1".to_string(),
                    phase: Some("p1".to_string()),
                    tags: vec![],
                    fqdn: "host1.example.com".to_string(),
                    reboot_policy: crate::nix::RebootPolicy::Auto,
                },
                target: DeployTarget {
                    user: Some("root".to_string()),
                    hostname: "host1.example.com".to_string(),
                    port: None,
                },
                store_path: StorePath::new("/nix/store/fake-host1".to_string()),
                activation_mode: ActivationMode::Switch,
                reboot_check: None,
            },
            PushPlan {
                metadata: HostMetadata {
                    name: "host2".to_string(),
                    phase: Some("p1".to_string()),
                    tags: vec![],
                    fqdn: "host2.example.com".to_string(),
                    reboot_policy: crate::nix::RebootPolicy::Auto,
                },
                target: DeployTarget {
                    user: Some("root".to_string()),
                    hostname: "host2.example.com".to_string(),
                    port: None,
                },
                store_path: StorePath::new("/nix/store/fake-host2".to_string()),
                activation_mode: ActivationMode::Switch,
                reboot_check: None,
            },
        ];

        let results = activate_plans_for_test(&deployer, plans, 2, &observer).unwrap();

        // Both hosts should have results
        assert_eq!(results.results.len(), 2);
        assert_eq!(results.success_count(), 1);
        assert_eq!(results.failure_count(), 1);

        // Check that one succeeded and one failed
        let hostnames: Vec<&str> = results.results.iter().map(|(n, _)| n.as_str()).collect();
        assert!(hostnames.contains(&"host1"));
        assert!(hostnames.contains(&"host2"));

        let host1_result = results.results.iter().find(|(n, _)| n == "host1").unwrap();
        let host2_result = results.results.iter().find(|(n, _)| n == "host2").unwrap();
        assert!(host1_result.1.success);
        assert!(!host2_result.1.success);

        // Phase should be marked as failed
        let events = observer.get_events();
        assert!(events.contains(&"phase_complete:p1:failed".to_string()));
    }

    #[test]
    fn test_push_error_message_includes_source() {
        // Test that when deployment fails, the error message includes the full error chain
        let flake = Flake::test_context();
        let nix_backend = MockNixBackend::new();
        let deployer =
            MockDeployBackend::new().with_activate_failures(vec!["fail.example.com".to_string()]);

        // Use a reboot detector that won't require reboots (empty config)
        let reboot_config = crate::backends::reboot::RebootDetectionConfig {
            critical_services: vec![],
            check_kernel: false,
            check_initrd: false,
            check_init: false,
        };
        let reboot_detector = crate::backends::reboot::SystemRebootBackend::new(reboot_config);
        let observer = crate::observer::NoopObserver;

        let hosts = vec![HostMetadata {
            name: "fail".to_string(),
            fqdn: "fail.example.com".to_string(),
            phase: Some("p1".to_string()),
            tags: vec![],
            reboot_policy: crate::nix::RebootPolicy::Auto,
        }];

        let config = PushConfiguration {
            mode: DeploymentMode::Switch,
            force: false,
            concurrency: 1,
            ignore_phases: false,
            quiet: false,
            ssh_user: None,
            allow_reboot: true,
        };

        // When activation fails, push() returns Err
        let result = {
            let push_ctx = PushContext::new(&nix_backend, &deployer, &reboot_detector, &config);
            push(&flake, &push_ctx, hosts, &observer)
        };

        // Push should fail due to activation failure
        assert!(
            result.is_err(),
            "Push should return error when activation fails"
        );

        // Verify the error message includes the source error chain
        let error = result.unwrap_err();

        // Build full error message including source chain
        let mut full_error = error.to_string();
        let mut source = std::error::Error::source(&error);
        while let Some(err) = source {
            full_error.push_str(&format!("\n  Caused by: {}", err));
            source = std::error::Error::source(err);
        }

        assert!(
            full_error.contains("fail"),
            "Error message should include the failed host name. Got: {}",
            full_error
        );
        assert!(
            full_error.contains("activation failed"),
            "Error message should include 'activation failed'. Got: {}",
            full_error
        );
    }

    #[test]
    fn test_push_error_message_format_with_activation_failure() {
        // Test the specific format of error messages with activation failures
        use std::collections::HashMap;

        let mut failures = HashMap::new();
        failures.insert(
            "test.example.com".to_string(),
            crate::backends::mock::FailureSpec {
                phase: "activate".to_string(),
                error: Some("systemd service failed to start".to_string()),
            },
        );

        let fixture = crate::backends::mock::DeploymentFixture {
            name: "Test error formatting".to_string(),
            description: None,
            failures,
            hosts: vec![],
        };

        let flake = Flake::test_context();
        let nix_backend = MockNixBackend::new();
        let deployer = MockDeployBackend::from_fixture(fixture);
        let reboot_config = crate::backends::reboot::RebootDetectionConfig {
            critical_services: vec![],
            check_kernel: false,
            check_initrd: false,
            check_init: false,
        };
        let reboot_detector = crate::backends::reboot::SystemRebootBackend::new(reboot_config);
        let observer = crate::observer::NoopObserver;

        let hosts = vec![HostMetadata {
            name: "test".to_string(),
            fqdn: "test.example.com".to_string(),
            phase: Some("p1".to_string()),
            tags: vec![],
            reboot_policy: crate::nix::RebootPolicy::Auto,
        }];

        let config = PushConfiguration {
            mode: DeploymentMode::Switch,
            force: false,
            concurrency: 1,
            ignore_phases: false,
            quiet: false,
            ssh_user: None,
            allow_reboot: true,
        };

        // When activation fails, push() returns Err
        let result = {
            let push_ctx = PushContext::new(&nix_backend, &deployer, &reboot_detector, &config);
            push(&flake, &push_ctx, hosts, &observer)
        };

        // Push should fail due to activation failure
        assert!(
            result.is_err(),
            "Push should return error when activation fails"
        );

        // Verify the error message includes expected details
        let error = result.unwrap_err();

        // Build full error message including source chain
        let mut full_error = error.to_string();
        let mut source = std::error::Error::source(&error);
        while let Some(err) = source {
            full_error.push_str(&format!("\n  Caused by: {}", err));
            source = std::error::Error::source(err);
        }

        assert!(
            full_error.contains("test"),
            "Error message should include the failed host name. Got: {}",
            full_error
        );
        assert!(
            full_error.contains("systemd service failed to start"),
            "Error message should include the specific error details. Got: {}",
            full_error
        );
    }

    // ============================================================================
    // Integration Tests for push() Function
    // ============================================================================

    #[test]
    fn test_push_full_workflow_single_phase_success() {
        // Test the complete workflow: build → copy → plan → activate for a single phase
        let flake = Flake::test_context();
        let nix_backend = MockNixBackend::new();
        let deployer = MockDeployBackend::new();
        let reboot_detector = SystemRebootBackend::default();
        let observer = TestObserver::new();

        let hosts = vec![
            HostMetadata {
                name: "web1".to_string(),
                fqdn: "web1.example.com".to_string(),
                phase: Some("web".to_string()),
                tags: vec![],
                reboot_policy: crate::nix::RebootPolicy::Never, // No reboot needed
            },
            HostMetadata {
                name: "web2".to_string(),
                fqdn: "web2.example.com".to_string(),
                phase: Some("web".to_string()),
                tags: vec![],
                reboot_policy: crate::nix::RebootPolicy::Never,
            },
        ];

        let config = PushConfiguration {
            mode: DeploymentMode::Switch,
            force: false,
            concurrency: 2,
            ignore_phases: false,
            quiet: false,
            ssh_user: None,
            allow_reboot: true,
        };

        let results = {
            let push_ctx = PushContext::new(&nix_backend, &deployer, &reboot_detector, &config);
            push(&flake, &push_ctx, hosts, &observer)
        }
        .unwrap();

        // Verify both hosts succeeded
        assert_eq!(results.success_count(), 2);
        assert_eq!(results.failure_count(), 0);

        // Verify observer events fired in correct order
        let events = observer.get_events();

        // Build step events
        assert!(events.contains(&"build_step_start:web:2".to_string()));
        assert!(events.contains(&"build_start:web1".to_string()));
        assert!(events.contains(&"build_complete:web1:ok".to_string()));
        assert!(events.contains(&"build_start:web2".to_string()));
        assert!(events.contains(&"build_complete:web2:ok".to_string()));

        // Copy step events
        assert!(events.contains(&"copy_step_start:web:2".to_string()));
        assert!(events.contains(&"copy_start:web1".to_string()));
        assert!(events.contains(&"copy_complete:web1:ok".to_string()));
        assert!(events.contains(&"copy_start:web2".to_string()));
        assert!(events.contains(&"copy_complete:web2:ok".to_string()));

        // Activation phase events
        assert!(events.contains(&"phase_start:web:2".to_string()));
        assert!(events.contains(&"deploy_start:web1".to_string()));
        assert!(events.contains(&"deploy_complete:web1:ok".to_string()));
        assert!(events.contains(&"deploy_start:web2".to_string()));
        assert!(events.contains(&"deploy_complete:web2:ok".to_string()));
        assert!(events.contains(&"phase_complete:web:ok".to_string()));
    }

    #[test]
    fn test_push_full_workflow_multi_phase_success() {
        // Test multiple phases deploying sequentially with all steps
        let flake = Flake::test_context();
        let nix_backend = MockNixBackend::new();
        let deployer = MockDeployBackend::new();
        let reboot_detector = SystemRebootBackend::default();
        let observer = TestObserver::new();

        let hosts = vec![
            HostMetadata {
                name: "db1".to_string(),
                fqdn: "db1.example.com".to_string(),
                phase: Some("database".to_string()),
                tags: vec![],
                reboot_policy: crate::nix::RebootPolicy::Never,
            },
            HostMetadata {
                name: "web1".to_string(),
                fqdn: "web1.example.com".to_string(),
                phase: Some("web".to_string()),
                tags: vec![],
                reboot_policy: crate::nix::RebootPolicy::Never,
            },
        ];

        let config = PushConfiguration {
            mode: DeploymentMode::Switch,
            force: false,
            concurrency: 2,
            ignore_phases: false,
            quiet: false,
            ssh_user: None,
            allow_reboot: true,
        };

        let results = {
            let push_ctx = PushContext::new(&nix_backend, &deployer, &reboot_detector, &config);
            push(&flake, &push_ctx, hosts, &observer)
        }
        .unwrap();

        assert_eq!(results.success_count(), 2);

        let events = observer.get_events();

        // Database phase should complete before web phase starts
        let db_phase_start = events
            .iter()
            .position(|e| e == "build_step_start:database:1")
            .expect("database build should start");
        let db_phase_complete = events
            .iter()
            .position(|e| e == "phase_complete:database:ok")
            .expect("database activation should complete");
        let web_phase_start = events
            .iter()
            .position(|e| e == "build_step_start:web:1")
            .expect("web build should start");

        assert!(db_phase_start < db_phase_complete);
        assert!(db_phase_complete < web_phase_start);
    }

    #[test]
    fn test_push_build_failure_stops_deployment() {
        // Build fails for one host, should stop immediately with no copy/activate
        let flake = Flake::test_context();
        let nix_backend = MockNixBackend::new().with_build_failures(vec!["fail-host".to_string()]);
        let deployer = MockDeployBackend::new();
        let reboot_detector = SystemRebootBackend::default();
        let observer = TestObserver::new();

        let hosts = vec![HostMetadata {
            name: "fail-host".to_string(),
            fqdn: "fail-host.example.com".to_string(),
            phase: None,
            tags: vec![],
            reboot_policy: crate::nix::RebootPolicy::Never,
        }];

        let config = PushConfiguration {
            mode: DeploymentMode::Switch,
            force: false,
            concurrency: 1,
            ignore_phases: false,
            quiet: false,
            ssh_user: None,
            allow_reboot: true,
        };

        let result = {
            let push_ctx = PushContext::new(&nix_backend, &deployer, &reboot_detector, &config);
            push(&flake, &push_ctx, hosts, &observer)
        };

        // Should fail during build
        assert!(result.is_err());

        let events = observer.get_events();

        // Phase lifecycle events should be present
        assert!(events.contains(&"phase_start:default:1".to_string()));
        assert!(events.contains(&"phase_complete:default:failed".to_string()));

        // Build events should be present
        assert!(events.contains(&"build_step_start:default:1".to_string()));
        assert!(events.contains(&"build_start:fail-host".to_string()));
        assert!(events.contains(&"build_complete:fail-host:fail".to_string()));
        assert!(events.contains(&"build_step_complete:default:fail".to_string()));

        // NO copy/activate/reboot events should fire (build failed before these steps)
        assert!(!events.iter().any(|e| e.starts_with("copy_")));
        assert!(!events.iter().any(|e| e.starts_with("deploy_")));
        assert!(!events.iter().any(|e| e.starts_with("reboot_")));
    }

    #[test]
    fn test_push_copy_failure_stops_deployment() {
        // Copy fails for one host after successful build
        let flake = Flake::test_context();
        let nix_backend = MockNixBackend::new();
        let deployer =
            MockDeployBackend::new().with_copy_failures(vec!["fail-copy.example.com".to_string()]);
        let reboot_detector = SystemRebootBackend::default();
        let observer = TestObserver::new();

        let hosts = vec![HostMetadata {
            name: "fail-copy".to_string(),
            fqdn: "fail-copy.example.com".to_string(),
            phase: None,
            tags: vec![],
            reboot_policy: crate::nix::RebootPolicy::Never,
        }];

        let config = PushConfiguration {
            mode: DeploymentMode::Switch,
            force: false,
            concurrency: 1,
            ignore_phases: false,
            quiet: false,
            ssh_user: None,
            allow_reboot: true,
        };

        let result = {
            let push_ctx = PushContext::new(&nix_backend, &deployer, &reboot_detector, &config);
            push(&flake, &push_ctx, hosts, &observer)
        };

        // Should fail during copy
        assert!(result.is_err());

        let events = observer.get_events();

        // Phase lifecycle events should be present
        assert!(events.contains(&"phase_start:default:1".to_string()));
        assert!(events.contains(&"phase_complete:default:failed".to_string()));

        // Build should succeed
        assert!(events.contains(&"build_step_start:default:1".to_string()));
        assert!(events.contains(&"build_start:fail-copy".to_string()));
        assert!(events.contains(&"build_complete:fail-copy:ok".to_string()));
        assert!(events.contains(&"build_step_complete:default:ok".to_string()));

        // Copy should fail
        assert!(events.contains(&"copy_step_start:default:1".to_string()));
        assert!(events.contains(&"copy_start:fail-copy".to_string()));
        assert!(events.contains(&"copy_complete:fail-copy:fail".to_string()));
        assert!(events.contains(&"copy_step_complete:default:fail".to_string()));

        // NO activate/reboot events (copy failed before these steps)
        assert!(!events.iter().any(|e| e.starts_with("deploy_")));
        assert!(!events.iter().any(|e| e.starts_with("reboot_")));
    }

    #[test]
    fn test_push_copy_observer_hooks_called() {
        // Verify copy observer hooks fire in correct order
        let flake = Flake::test_context();
        let nix_backend = MockNixBackend::new();
        let deployer = MockDeployBackend::new();
        let reboot_detector = SystemRebootBackend::default();
        let observer = TestObserver::new();

        let hosts = vec![
            HostMetadata {
                name: "host1".to_string(),
                fqdn: "host1.example.com".to_string(),
                phase: Some("p1".to_string()),
                tags: vec![],
                reboot_policy: crate::nix::RebootPolicy::Never,
            },
            HostMetadata {
                name: "host2".to_string(),
                fqdn: "host2.example.com".to_string(),
                phase: Some("p1".to_string()),
                tags: vec![],
                reboot_policy: crate::nix::RebootPolicy::Never,
            },
        ];

        let config = PushConfiguration {
            mode: DeploymentMode::Switch,
            force: false,
            concurrency: 2,
            ignore_phases: false,
            quiet: false,
            ssh_user: None,
            allow_reboot: true,
        };

        {
            let push_ctx = PushContext::new(&nix_backend, &deployer, &reboot_detector, &config);
            push(&flake, &push_ctx, hosts, &observer)
        }
        .unwrap();

        let events = observer.get_events();

        // Verify copy sequence: copy_step_start → copy_start → copy_complete
        let copy_phase_idx = events
            .iter()
            .position(|e| e == "copy_step_start:p1:2")
            .unwrap();
        let copy1_start_idx = events.iter().position(|e| e == "copy_start:host1").unwrap();
        let copy1_complete_idx = events
            .iter()
            .position(|e| e == "copy_complete:host1:ok")
            .unwrap();
        let copy2_start_idx = events.iter().position(|e| e == "copy_start:host2").unwrap();
        let copy2_complete_idx = events
            .iter()
            .position(|e| e == "copy_complete:host2:ok")
            .unwrap();

        assert!(copy_phase_idx < copy1_start_idx);
        assert!(copy1_start_idx < copy1_complete_idx);
        assert!(copy_phase_idx < copy2_start_idx);
        assert!(copy2_start_idx < copy2_complete_idx);
    }

    #[test]
    fn test_push_build_observer_hooks_called() {
        // Verify build observer hooks fire in correct order
        let flake = Flake::test_context();
        let nix_backend = MockNixBackend::new();
        let deployer = MockDeployBackend::new();
        let reboot_detector = SystemRebootBackend::default();
        let observer = TestObserver::new();

        let hosts = vec![
            HostMetadata {
                name: "host1".to_string(),
                fqdn: "host1.example.com".to_string(),
                phase: Some("p1".to_string()),
                tags: vec![],
                reboot_policy: crate::nix::RebootPolicy::Never,
            },
            HostMetadata {
                name: "host2".to_string(),
                fqdn: "host2.example.com".to_string(),
                phase: Some("p1".to_string()),
                tags: vec![],
                reboot_policy: crate::nix::RebootPolicy::Never,
            },
        ];

        let config = PushConfiguration {
            mode: DeploymentMode::Switch,
            force: false,
            concurrency: 2,
            ignore_phases: false,
            quiet: false,
            ssh_user: None,
            allow_reboot: true,
        };

        {
            let push_ctx = PushContext::new(&nix_backend, &deployer, &reboot_detector, &config);
            push(&flake, &push_ctx, hosts, &observer)
        }
        .unwrap();

        let events = observer.get_events();

        // Verify build sequence: build_step_start → build_start → build_complete
        let build_phase_idx = events
            .iter()
            .position(|e| e == "build_step_start:p1:2")
            .unwrap();
        let build1_start_idx = events
            .iter()
            .position(|e| e == "build_start:host1")
            .unwrap();
        let build1_complete_idx = events
            .iter()
            .position(|e| e == "build_complete:host1:ok")
            .unwrap();
        let build2_start_idx = events
            .iter()
            .position(|e| e == "build_start:host2")
            .unwrap();
        let build2_complete_idx = events
            .iter()
            .position(|e| e == "build_complete:host2:ok")
            .unwrap();

        assert!(build_phase_idx < build1_start_idx);
        assert!(build1_start_idx < build1_complete_idx);
        assert!(build_phase_idx < build2_start_idx);
        assert!(build2_start_idx < build2_complete_idx);
    }

    #[test]
    fn test_push_build_partial_failure() {
        // Build fails for host 2 of 2 in a phase
        let flake = Flake::test_context();
        let nix_backend = MockNixBackend::new().with_build_failures(vec!["host2".to_string()]);
        let deployer = MockDeployBackend::new();
        let reboot_detector = SystemRebootBackend::default();
        let observer = TestObserver::new();

        let hosts = vec![
            HostMetadata {
                name: "host1".to_string(),
                fqdn: "host1.example.com".to_string(),
                phase: Some("p1".to_string()),
                tags: vec![],
                reboot_policy: crate::nix::RebootPolicy::Never,
            },
            HostMetadata {
                name: "host2".to_string(),
                fqdn: "host2.example.com".to_string(),
                phase: Some("p1".to_string()),
                tags: vec![],
                reboot_policy: crate::nix::RebootPolicy::Never,
            },
        ];

        let config = PushConfiguration {
            mode: DeploymentMode::Switch,
            force: false,
            concurrency: 2,
            ignore_phases: false,
            quiet: false,
            ssh_user: None,
            allow_reboot: true,
        };

        let result = {
            let push_ctx = PushContext::new(&nix_backend, &deployer, &reboot_detector, &config);
            push(&flake, &push_ctx, hosts, &observer)
        };

        assert!(result.is_err());

        let events = observer.get_events();

        // host1 build should succeed
        assert!(events.contains(&"build_complete:host1:ok".to_string()));

        // host2 build should fail
        assert!(events.contains(&"build_complete:host2:fail".to_string()));

        // No copy should happen for either host
        assert!(!events.iter().any(|e| e.starts_with("copy_")));
    }

    #[test]
    fn test_push_ignore_phases_flag() {
        // Test that ignore_phases groups all hosts into "_all" phase
        let flake = Flake::test_context();
        let nix_backend = MockNixBackend::new();
        let deployer = MockDeployBackend::new();
        let reboot_detector = SystemRebootBackend::default();
        let observer = TestObserver::new();

        let hosts = vec![
            HostMetadata {
                name: "db1".to_string(),
                fqdn: "db1.example.com".to_string(),
                phase: Some("database".to_string()),
                tags: vec![],
                reboot_policy: crate::nix::RebootPolicy::Never,
            },
            HostMetadata {
                name: "web1".to_string(),
                fqdn: "web1.example.com".to_string(),
                phase: Some("web".to_string()),
                tags: vec![],
                reboot_policy: crate::nix::RebootPolicy::Never,
            },
        ];

        let config = PushConfiguration {
            mode: DeploymentMode::Switch,
            force: false,
            concurrency: 2,
            ignore_phases: true, // Ignore phase ordering
            quiet: false,
            ssh_user: None,
            allow_reboot: true,
        };

        let results = {
            let push_ctx = PushContext::new(&nix_backend, &deployer, &reboot_detector, &config);
            push(&flake, &push_ctx, hosts, &observer)
        }
        .unwrap();

        assert_eq!(results.success_count(), 2);

        let events = observer.get_events();

        // Both hosts should be in "_all" phase
        assert!(events.contains(&"build_step_start:_all:2".to_string()));
        assert!(events.contains(&"copy_step_start:_all:2".to_string()));
        assert!(events.contains(&"phase_start:_all:2".to_string()));

        // Should NOT have separate database/web phases
        assert!(!events.iter().any(|e| e.contains(":database:")));
        assert!(!events.iter().any(|e| e.contains(":web:")));
    }

    #[test]
    fn test_push_phase_failure_stops_next_phase() {
        // Phase 1 activation fails, Phase 2 should never start
        let flake = Flake::test_context();
        let nix_backend = MockNixBackend::new();
        let deployer = MockDeployBackend::new()
            .with_activate_failures(vec!["phase1-host.example.com".to_string()]);
        let reboot_detector = SystemRebootBackend::default();
        let observer = TestObserver::new();

        let hosts = vec![
            HostMetadata {
                name: "phase1-host".to_string(),
                fqdn: "phase1-host.example.com".to_string(),
                phase: Some("phase1".to_string()),
                tags: vec![],
                reboot_policy: crate::nix::RebootPolicy::Never,
            },
            HostMetadata {
                name: "phase2-host".to_string(),
                fqdn: "phase2-host.example.com".to_string(),
                phase: Some("phase2".to_string()),
                tags: vec![],
                reboot_policy: crate::nix::RebootPolicy::Never,
            },
        ];

        let config = PushConfiguration {
            mode: DeploymentMode::Switch,
            force: false,
            concurrency: 1,
            ignore_phases: false,
            quiet: false,
            ssh_user: None,
            allow_reboot: true,
        };

        // When activation fails, push() returns Err but observer events still fire
        let result = {
            let push_ctx = PushContext::new(&nix_backend, &deployer, &reboot_detector, &config);
            push(&flake, &push_ctx, hosts, &observer)
        };

        // Push should fail due to phase1 activation failure
        assert!(
            result.is_err(),
            "Push should return error when activation fails"
        );

        let events = observer.get_events();

        // Phase1 should have all events (build, copy, deploy)
        assert!(events.iter().any(|e| e.contains("phase1")));
        assert!(events.iter().any(|e| e.contains("phase1-host")));

        // Phase2 should have NO events (build, copy, or activate)
        assert!(
            !events.iter().any(|e| e.contains("phase2")),
            "Phase2 should not start after phase1 failure"
        );
        assert!(
            !events.iter().any(|e| e.contains("phase2-host")),
            "Phase2 host should not be processed"
        );
    }

    #[test]
    fn test_push_reboot_execution_success() {
        // Test that reboot is executed when required and observer hooks are called
        let flake = Flake::test_context();
        let nix_backend = MockNixBackend::new();
        let deployer = MockDeployBackend::new();

        // Configure reboot detection to require reboot
        use crate::backends::mock::MockRebootBackend;
        use crate::backends::reboot::RebootReason;
        let mut reboot_required = std::collections::HashMap::new();
        reboot_required.insert(
            "reboot-host.example.com".to_string(),
            RebootReason::KernelChanged,
        );
        let reboot_backend = MockRebootBackend::new().with_reboot_required(reboot_required.clone());

        let observer = TestObserver::new();

        let hosts = vec![make_host_metadata(
            "reboot-host",
            Some("default"),
            crate::nix::RebootPolicy::Auto,
        )];

        let config = PushConfiguration {
            mode: DeploymentMode::Auto, // Auto mode will detect reboot requirement
            force: false,
            concurrency: 1,
            ignore_phases: false,
            quiet: false,
            ssh_user: None,
            allow_reboot: true,
        };

        let results = {
            let push_ctx = PushContext::new(&nix_backend, &deployer, &reboot_backend, &config);
            push(&flake, &push_ctx, hosts, &observer)
        }
        .unwrap();

        // Should succeed
        assert!(results.all_succeeded());
        assert_eq!(results.success_count(), 1);
        assert_eq!(results.failure_count(), 0);

        // Check observer events - should include reboot events
        let events = observer.get_events();

        // Should have reboot events
        assert!(
            events.iter().any(|e| e == "reboot_start:reboot-host"),
            "Missing reboot_start event. Events: {:?}",
            events
        );
        assert!(
            events.iter().any(|e| e == "reboot_complete:reboot-host:ok"),
            "Missing reboot_complete event. Events: {:?}",
            events
        );

        // Verify event sequence: build -> copy -> deploy -> reboot
        let reboot_start_idx = events
            .iter()
            .position(|e| e == "reboot_start:reboot-host")
            .unwrap();
        let deploy_complete_idx = events
            .iter()
            .position(|e| e.starts_with("deploy_complete:reboot-host"))
            .unwrap();

        assert!(
            deploy_complete_idx < reboot_start_idx,
            "Reboot should happen after deployment"
        );
    }

    #[test]
    fn test_push_reboot_execution_failure() {
        // Test that reboot failures are properly handled and reported
        let flake = Flake::test_context();
        let nix_backend = MockNixBackend::new();
        let deployer = MockDeployBackend::new();

        // Configure reboot to be required but fail during execution
        use crate::backends::mock::MockRebootBackend;
        use crate::backends::reboot::RebootReason;
        let mut reboot_required = std::collections::HashMap::new();
        reboot_required.insert(
            "fail-reboot.example.com".to_string(),
            RebootReason::InitrdChanged,
        );
        let reboot_backend = MockRebootBackend::new()
            .with_reboot_required(reboot_required)
            .with_reboot_failures(vec!["fail-reboot.example.com".to_string()]);

        let observer = TestObserver::new();

        let hosts = vec![make_host_metadata(
            "fail-reboot",
            Some("default"),
            crate::nix::RebootPolicy::Auto,
        )];

        let config = PushConfiguration {
            mode: DeploymentMode::Auto,
            force: false,
            concurrency: 1,
            ignore_phases: false,
            quiet: false,
            ssh_user: None,
            allow_reboot: true,
        };

        let results = {
            let push_ctx = PushContext::new(&nix_backend, &deployer, &reboot_backend, &config);
            push(&flake, &push_ctx, hosts, &observer)
        }
        .unwrap();

        // Should report failure due to reboot failure
        assert!(!results.all_succeeded());
        assert_eq!(results.failure_count(), 1);

        // Check that deployment itself succeeded but reboot failed
        let (hostname, result) = &results.results[0];
        assert_eq!(hostname, "fail-reboot");
        assert!(!result.success);
        assert!(
            result.message.contains("reboot failed"),
            "Error should mention reboot failure: {}",
            result.message
        );

        // Check observer events
        let events = observer.get_events();

        // Should have both reboot_start and reboot_complete:fail
        assert!(
            events.iter().any(|e| e == "reboot_start:fail-reboot"),
            "Missing reboot_start event"
        );
        assert!(
            events
                .iter()
                .any(|e| e == "reboot_complete:fail-reboot:fail"),
            "Missing reboot_complete:fail event"
        );
    }

    #[test]
    fn test_push_no_reboot_when_not_required() {
        // Test that reboot hooks are NOT called when reboot is not required
        let flake = Flake::test_context();
        let nix_backend = MockNixBackend::new();
        let deployer = MockDeployBackend::new();

        // No reboot required (empty HashMap)
        use crate::backends::mock::MockRebootBackend;
        let reboot_backend = MockRebootBackend::new();

        let observer = TestObserver::new();

        let hosts = vec![make_host_metadata(
            "no-reboot",
            Some("default"),
            crate::nix::RebootPolicy::Auto,
        )];

        let config = PushConfiguration {
            mode: DeploymentMode::Auto,
            force: false,
            concurrency: 1,
            ignore_phases: false,
            quiet: false,
            ssh_user: None,
            allow_reboot: true,
        };

        let results = {
            let push_ctx = PushContext::new(&nix_backend, &deployer, &reboot_backend, &config);
            push(&flake, &push_ctx, hosts, &observer)
        }
        .unwrap();

        // Should succeed
        assert!(results.all_succeeded());
        assert_eq!(results.success_count(), 1);

        // Check observer events - should NOT include reboot events
        let events = observer.get_events();

        // Should NOT have reboot events
        assert!(
            !events.iter().any(|e| e.contains("reboot_start")),
            "Should not have reboot_start when reboot not required"
        );
        assert!(
            !events.iter().any(|e| e.contains("reboot_complete")),
            "Should not have reboot_complete when reboot not required"
        );

        // Should have completed deployment
        assert!(events.iter().any(|e| e.starts_with("deploy_complete")));
    }

    #[test]
    fn test_push_reboot_with_boot_mode() {
        // Test explicit Boot mode with reboot required
        let flake = Flake::test_context();
        let nix_backend = MockNixBackend::new();
        let deployer = MockDeployBackend::new();

        // Configure reboot required
        use crate::backends::mock::MockRebootBackend;
        use crate::backends::reboot::RebootReason;
        let mut reboot_required = std::collections::HashMap::new();
        reboot_required.insert(
            "boot-host.example.com".to_string(),
            RebootReason::InitChanged,
        );
        let reboot_backend = MockRebootBackend::new().with_reboot_required(reboot_required);

        let observer = TestObserver::new();

        let hosts = vec![make_host_metadata(
            "boot-host",
            Some("default"),
            crate::nix::RebootPolicy::Auto,
        )];

        let config = PushConfiguration {
            mode: DeploymentMode::Boot, // Explicit Boot mode
            force: false,
            concurrency: 1,
            ignore_phases: false,
            quiet: false,
            ssh_user: None,
            allow_reboot: true,
        };

        let results = {
            let push_ctx = PushContext::new(&nix_backend, &deployer, &reboot_backend, &config);
            push(&flake, &push_ctx, hosts, &observer)
        }
        .unwrap();

        // Should succeed
        assert!(results.all_succeeded());

        // Check that reboot was executed
        let events = observer.get_events();
        assert!(
            events.iter().any(|e| e.starts_with("reboot_start")),
            "Boot mode with reboot required should execute reboot"
        );
        assert!(events.iter().any(|e| e.starts_with("reboot_complete")));
    }

    #[test]
    fn test_push_reboot_policy_always() {
        // Test that RebootPolicy::Always forces reboot even if not detected
        let flake = Flake::test_context();
        let nix_backend = MockNixBackend::new();
        let deployer = MockDeployBackend::new();

        // No reboot detected, but policy is Always
        use crate::backends::mock::MockRebootBackend;
        let reboot_backend = MockRebootBackend::new(); // Empty - no reboot required

        let observer = TestObserver::new();

        let hosts = vec![make_host_metadata(
            "always-reboot",
            Some("default"),
            crate::nix::RebootPolicy::Always, // Always reboot
        )];

        let config = PushConfiguration {
            mode: DeploymentMode::Auto,
            force: false,
            concurrency: 1,
            ignore_phases: false,
            quiet: false,
            ssh_user: None,
            allow_reboot: true,
        };

        let results = {
            let push_ctx = PushContext::new(&nix_backend, &deployer, &reboot_backend, &config);
            push(&flake, &push_ctx, hosts, &observer)
        }
        .unwrap();

        // Should succeed
        assert!(results.all_succeeded());

        // Check that reboot was executed despite no detection
        let events = observer.get_events();
        assert!(
            events.iter().any(|e| e == "reboot_start:always-reboot"),
            "RebootPolicy::Always should execute reboot. Events: {:?}",
            events
        );
        assert!(events
            .iter()
            .any(|e| e == "reboot_complete:always-reboot:ok"));
    }

    #[test]
    fn test_push_reboot_policy_never() {
        // Test that RebootPolicy::Never prevents reboot even if detected
        let flake = Flake::test_context();
        let nix_backend = MockNixBackend::new();
        let deployer = MockDeployBackend::new();

        // Reboot required but policy is Never
        use crate::backends::mock::MockRebootBackend;
        use crate::backends::reboot::RebootReason;
        let mut reboot_required = std::collections::HashMap::new();
        reboot_required.insert(
            "never-reboot.example.com".to_string(),
            RebootReason::KernelChanged,
        );
        let reboot_backend = MockRebootBackend::new().with_reboot_required(reboot_required);

        let observer = TestObserver::new();

        let hosts = vec![make_host_metadata(
            "never-reboot",
            Some("default"),
            crate::nix::RebootPolicy::Never, // Never reboot
        )];

        let config = PushConfiguration {
            mode: DeploymentMode::Auto,
            force: false,
            concurrency: 1,
            ignore_phases: false,
            quiet: false,
            ssh_user: None,
            allow_reboot: true,
        };

        let results = {
            let push_ctx = PushContext::new(&nix_backend, &deployer, &reboot_backend, &config);
            push(&flake, &push_ctx, hosts, &observer)
        }
        .unwrap();

        // Should succeed
        assert!(results.all_succeeded());

        // Check that reboot was NOT executed despite detection
        let events = observer.get_events();
        assert!(
            !events.iter().any(|e| e.contains("reboot_start")),
            "RebootPolicy::Never should prevent reboot"
        );
        assert!(!events.iter().any(|e| e.contains("reboot_complete")));

        // Should still have deployed successfully
        assert!(events.iter().any(|e| e.starts_with("deploy_complete")));
    }

    #[test]
    fn test_push_no_reboot_flag_overrides_always_policy() {
        // Test that allow_reboot=false prevents reboot even with RebootPolicy::Always
        let flake = Flake::test_context();
        let nix_backend = MockNixBackend::new();
        let deployer = MockDeployBackend::new();

        // Policy is Always, but user says --no-reboot
        use crate::backends::mock::MockRebootBackend;
        let reboot_backend = MockRebootBackend::new();

        let observer = TestObserver::new();

        let hosts = vec![make_host_metadata(
            "always-reboot",
            Some("default"),
            crate::nix::RebootPolicy::Always, // Policy says always reboot
        )];

        let config = PushConfiguration {
            mode: DeploymentMode::Auto,
            force: false,
            concurrency: 1,
            ignore_phases: false,
            quiet: false,
            ssh_user: None,
            allow_reboot: false, // User says --no-reboot
        };

        let results = {
            let push_ctx = PushContext::new(&nix_backend, &deployer, &reboot_backend, &config);
            push(&flake, &push_ctx, hosts, &observer)
        }
        .unwrap();

        // Should succeed
        assert!(results.all_succeeded());

        // Check that reboot was NOT executed despite policy
        let events = observer.get_events();
        assert!(
            !events.iter().any(|e| e.contains("reboot_start")),
            "allow_reboot=false should prevent reboot even with RebootPolicy::Always"
        );
        assert!(!events.iter().any(|e| e.contains("reboot_complete")));

        // Should still have deployed successfully
        assert!(events.iter().any(|e| e.starts_with("deploy_complete")));
    }

    #[test]
    fn test_push_no_reboot_flag_with_detected_reboot() {
        // Test that allow_reboot=false prevents reboot even when detection says it's needed
        let flake = Flake::test_context();
        let nix_backend = MockNixBackend::new();
        let deployer = MockDeployBackend::new();

        // Reboot detected as required but user says --no-reboot
        use crate::backends::mock::MockRebootBackend;
        use crate::backends::reboot::RebootReason;
        let mut reboot_required = std::collections::HashMap::new();
        reboot_required.insert(
            "needs-reboot.example.com".to_string(),
            RebootReason::KernelChanged,
        );
        let reboot_backend = MockRebootBackend::new().with_reboot_required(reboot_required);

        let observer = TestObserver::new();

        let hosts = vec![make_host_metadata(
            "needs-reboot",
            Some("default"),
            crate::nix::RebootPolicy::Auto, // Auto policy
        )];

        let config = PushConfiguration {
            mode: DeploymentMode::Auto,
            force: false,
            concurrency: 1,
            ignore_phases: false,
            quiet: false,
            ssh_user: None,
            allow_reboot: false, // User says --no-reboot
        };

        let results = {
            let push_ctx = PushContext::new(&nix_backend, &deployer, &reboot_backend, &config);
            push(&flake, &push_ctx, hosts, &observer)
        }
        .unwrap();

        // Should succeed (uses Boot mode but doesn't reboot)
        assert!(results.all_succeeded());

        // Check that reboot was NOT executed despite detection
        let events = observer.get_events();
        assert!(
            !events.iter().any(|e| e.contains("reboot_start")),
            "allow_reboot=false should prevent reboot even when detection says reboot needed"
        );
        assert!(!events.iter().any(|e| e.contains("reboot_complete")));

        // Should still have deployed successfully (Boot mode without reboot)
        assert!(events.iter().any(|e| e.starts_with("deploy_complete")));
    }
}
