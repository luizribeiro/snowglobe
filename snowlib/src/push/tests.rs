//! Tests for push deployment

use super::planning::create_deployment_plans;
use super::*;
use crate::backends::deploy::DeployTarget;
use crate::backends::mock::{MockDeployBackend, MockNixBackend};
use crate::nix::{ActivationMode, Flake, HostMetadata, StorePath, SystemType};

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
        system_type: SystemType::NixOS,
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

/// Helper function to activate hosts (simplified for tests)
fn activate_plans_for_test(
    deploy_backend: &dyn crate::backends::deploy::DeployBackend,
    plans: Vec<PushPlan>,
    _concurrency: usize,
    observer: &dyn crate::observer::PushObserver,
) -> Result<PushResults> {
    use crate::backends::deploy::DeployResult;

    // Get the phase from the first plan (all plans in tests have the same phase)
    let phase = plans
        .first()
        .and_then(|p| p.metadata.phase.as_ref())
        .cloned()
        .unwrap_or_else(|| "default".to_string());

    // Collect hostnames for phase lifecycle notification
    let hostnames: Vec<String> = plans.iter().map(|p| p.metadata.name.clone()).collect();

    // Notify observer of phase start
    observer.on_phase_start(&phase, &hostnames);

    // Activate each plan
    let mut host_results = Vec::new();

    for plan in plans {
        let hostname = plan.metadata.name.clone();

        // Notify activation start
        observer.on_activation_start(&hostname);

        // Execute activation
        let result = deploy_backend
            .deploy(&plan.target, &plan.store_path, plan.activation_mode)
            .unwrap_or_else(|e| DeployResult::failure(e.to_string()));

        // Notify activation complete
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

    // Notify observer of phase completion
    observer.on_phase_complete(&phase, phase_result.all_succeeded());

    Ok(PushResults {
        phases: vec![phase_result],
        skipped_phases: vec![],
    })
}

#[test]
fn test_push_results_counts() {
    let results = PushResults {
        phases: vec![PhaseResult {
            phase: "default".to_string(),
            hosts: vec![
                HostDeploymentResult {
                    hostname: "host1".to_string(),
                    success: true,
                    failed_at_step: None,
                    activation_mode: Some(ActivationMode::Switch),
                    rebooted: false,
                    error_message: None,
                },
                HostDeploymentResult {
                    hostname: "host2".to_string(),
                    success: false,
                    failed_at_step: Some(PipelineStep::Activate),
                    activation_mode: None,
                    rebooted: false,
                    error_message: Some("failed".to_string()),
                },
                HostDeploymentResult {
                    hostname: "host3".to_string(),
                    success: true,
                    failed_at_step: None,
                    activation_mode: Some(ActivationMode::Switch),
                    rebooted: false,
                    error_message: None,
                },
            ],
        }],
        skipped_phases: vec![],
    };

    assert_eq!(results.success_count(), 2);
    assert_eq!(results.failure_count(), 1);
    assert!(!results.all_succeeded());
}

#[test]
fn test_push_results_all_succeeded() {
    let results = PushResults {
        phases: vec![PhaseResult {
            phase: "default".to_string(),
            hosts: vec![
                HostDeploymentResult {
                    hostname: "host1".to_string(),
                    success: true,
                    failed_at_step: None,
                    activation_mode: Some(ActivationMode::Switch),
                    rebooted: false,
                    error_message: None,
                },
                HostDeploymentResult {
                    hostname: "host2".to_string(),
                    success: true,
                    failed_at_step: None,
                    activation_mode: Some(ActivationMode::Switch),
                    rebooted: false,
                    error_message: None,
                },
            ],
        }],
        skipped_phases: vec![],
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
        ..Default::default()
    };
    let detector = SystemRebootBackend::new(config);

    let hosts = vec![make_host("test", None, crate::nix::RebootPolicy::Auto)];

    let (plans, blocked) =
        create_deployment_plans(hosts, &detector, PushStrategy::Auto, false).unwrap();

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
        create_deployment_plans(hosts, &detector, PushStrategy::DryRun, false).unwrap();

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
        create_deployment_plans(hosts, &detector, PushStrategy::Boot, false).unwrap();

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
        create_deployment_plans(hosts, &detector, PushStrategy::Test, false).unwrap();

    assert_eq!(plans.len(), 1);
    assert_eq!(blocked.len(), 0);
    assert_eq!(plans[0].activation_mode, ActivationMode::Test);
    assert!(plans[0].reboot_check.is_none());
}

#[test]
fn test_create_deployment_plans_switch_with_force() {
    // In reality, reboot would be detected, but with force flag it should still use Switch
    // Disable reboot checks to avoid SSH calls in unit tests
    let config = RebootDetectionConfig {
        critical_services: vec![],
        check_kernel: false,
        check_initrd: false,
        ..Default::default()
    };
    let detector = SystemRebootBackend::new(config);
    let hosts = vec![make_host("test", None, crate::nix::RebootPolicy::Auto)];

    let (plans, blocked) =
        create_deployment_plans(hosts, &detector, PushStrategy::Switch, true).unwrap();

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
        ..Default::default()
    };
    let detector = SystemRebootBackend::new(config);

    let hosts = vec![
        make_host("host1", None, crate::nix::RebootPolicy::Auto),
        make_host("host2", None, crate::nix::RebootPolicy::Auto),
        make_host("host3", None, crate::nix::RebootPolicy::Auto),
    ];

    let (plans, blocked) =
        create_deployment_plans(hosts, &detector, PushStrategy::Auto, false).unwrap();

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
        ..Default::default()
    };
    let detector = SystemRebootBackend::new(config);

    let hosts = vec![make_host("host1", None, crate::nix::RebootPolicy::Never)];

    let (plans, blocked) =
        create_deployment_plans(hosts, &detector, PushStrategy::Auto, false).unwrap();

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
        ..Default::default()
    };
    let detector = SystemRebootBackend::new(config);

    let hosts = vec![make_host("host1", None, crate::nix::RebootPolicy::Always)];

    let (plans, blocked) =
        create_deployment_plans(hosts, &detector, PushStrategy::Auto, false).unwrap();

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
        ..Default::default()
    };
    let detector = SystemRebootBackend::new(config);

    let hosts = vec![make_host("host1", None, crate::nix::RebootPolicy::Auto)];

    let (plans, blocked) =
        create_deployment_plans(hosts, &detector, PushStrategy::Auto, false).unwrap();

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
        ..Default::default()
    };
    let detector = SystemRebootBackend::new(config);

    let hosts = vec![make_host("host1", None, crate::nix::RebootPolicy::Never)];

    let (plans, blocked) =
        create_deployment_plans(hosts, &detector, PushStrategy::Switch, false).unwrap();

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
        ..Default::default()
    };
    let detector = SystemRebootBackend::new(config);

    let hosts = vec![make_host("host1", None, crate::nix::RebootPolicy::Always)];

    let (plans, blocked) =
        create_deployment_plans(hosts, &detector, PushStrategy::Switch, false).unwrap();

    assert_eq!(plans.len(), 1);
    assert_eq!(blocked.len(), 0);
    // Should use boot mode despite switch flag (policy overrides)
    assert_eq!(plans[0].activation_mode, ActivationMode::Boot);
    assert!(plans[0].reboot_check.as_ref().unwrap().required);
}

#[test]
fn test_force_overrides_never_policy_with_switch() {
    let config = RebootDetectionConfig {
        critical_services: vec![],
        check_kernel: false,
        check_initrd: false,
        ..Default::default()
    };
    let detector = SystemRebootBackend::new(config);

    let hosts = vec![make_host("host1", None, crate::nix::RebootPolicy::Never)];

    let (plans, blocked) =
        create_deployment_plans(hosts, &detector, PushStrategy::Switch, true).unwrap();

    assert_eq!(plans.len(), 1);
    assert_eq!(blocked.len(), 0);
    // Force flag should override 'never' policy and use switch mode
    assert_eq!(plans[0].activation_mode, ActivationMode::Switch);
}

#[test]
fn test_force_overrides_always_policy_with_switch() {
    let config = RebootDetectionConfig {
        critical_services: vec![],
        check_kernel: false,
        check_initrd: false,
        ..Default::default()
    };
    let detector = SystemRebootBackend::new(config);

    let hosts = vec![make_host("host1", None, crate::nix::RebootPolicy::Always)];

    let (plans, blocked) =
        create_deployment_plans(hosts, &detector, PushStrategy::Switch, true).unwrap();

    assert_eq!(plans.len(), 1);
    assert_eq!(blocked.len(), 0);
    // Force flag should override 'always' policy and use switch mode
    assert_eq!(plans[0].activation_mode, ActivationMode::Switch);
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
                system_type: SystemType::NixOS,
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
                system_type: SystemType::NixOS,
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
    assert!(events.contains(&"activation_start:host1".to_string()));
    assert!(events.contains(&"activation_start:host2".to_string()));
    assert!(events.contains(&"activation_complete:host1:ok".to_string()));
    assert!(events.contains(&"activation_complete:host2:ok".to_string()));
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
        ..Default::default()
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
            system_type: SystemType::NixOS,
        },
        HostMetadata {
            name: "host1".to_string(),
            fqdn: "host1.example.com".to_string(),
            phase: Some("p1".to_string()),
            tags: vec![],
            reboot_policy: crate::nix::RebootPolicy::Auto,
            system_type: SystemType::NixOS,
        },
    ];

    let config = PushConfiguration {
        strategy: PushStrategy::Switch,
        force: false,
        concurrency: 2,
        ignore_phases: false,
        output_mode: crate::process::OutputMode::Capture,
        ssh_user: None,
        allow_reboot: true,
    };

    let results = {
        let push_ctx = PushContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_detector,
            &config,
            &observer,
        );
        push(&push_ctx, hosts)
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
        ..Default::default()
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
            system_type: SystemType::NixOS,
        },
        HostMetadata {
            name: "host2".to_string(),
            fqdn: "host2.example.com".to_string(),
            phase: Some("p2".to_string()),
            tags: vec![],
            reboot_policy: crate::nix::RebootPolicy::Auto,
            system_type: SystemType::NixOS,
        },
    ];

    let config = PushConfiguration {
        strategy: PushStrategy::Switch,
        force: false,
        concurrency: 2,
        ignore_phases: false,
        output_mode: crate::process::OutputMode::Capture,
        ssh_user: None,
        allow_reboot: true,
    };

    // When activation fails, push() returns Err
    let result = {
        let push_ctx = PushContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_detector,
            &config,
            &observer,
        );
        push(&push_ctx, hosts)
    };

    // Push should succeed but with failures and skipped phases
    let results = result.expect("Push should succeed structurally");
    assert!(!results.all_succeeded(), "Push should have failures");
    assert_eq!(results.failure_count(), 1);
    assert_eq!(
        results.skipped_phases.len(),
        1,
        "Phase p2 should be skipped"
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
            system_type: SystemType::NixOS,
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
            system_type: SystemType::NixOS,
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
    let deployer = MockDeployBackend::new().with_failures(vec!["host2.example.com".to_string()]);
    let observer = TestObserver::new();

    let plans = vec![
        PushPlan {
            metadata: HostMetadata {
                name: "host1".to_string(),
                phase: Some("p1".to_string()),
                tags: vec![],
                fqdn: "host1.example.com".to_string(),
                reboot_policy: crate::nix::RebootPolicy::Auto,
                system_type: SystemType::NixOS,
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
                system_type: SystemType::NixOS,
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
    assert_eq!(results.phases[0].hosts.len(), 2);
    assert_eq!(results.success_count(), 1);
    assert_eq!(results.failure_count(), 1);

    // Check that one succeeded and one failed
    let hostnames: Vec<&str> = results.phases[0]
        .hosts
        .iter()
        .map(|h| h.hostname.as_str())
        .collect();
    assert!(hostnames.contains(&"host1"));
    assert!(hostnames.contains(&"host2"));

    let host1_result = results.phases[0]
        .hosts
        .iter()
        .find(|h| h.hostname == "host1")
        .unwrap();
    let host2_result = results.phases[0]
        .hosts
        .iter()
        .find(|h| h.hostname == "host2")
        .unwrap();
    assert!(host1_result.success);
    assert!(!host2_result.success);

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
        ..Default::default()
    };
    let reboot_detector = crate::backends::reboot::SystemRebootBackend::new(reboot_config);
    let observer = crate::observer::NoopObserver;

    let hosts = vec![HostMetadata {
        name: "fail".to_string(),
        fqdn: "fail.example.com".to_string(),
        phase: Some("p1".to_string()),
        tags: vec![],
        reboot_policy: crate::nix::RebootPolicy::Auto,
        system_type: SystemType::NixOS,
    }];

    let config = PushConfiguration {
        strategy: PushStrategy::Switch,
        force: false,
        concurrency: 1,
        ignore_phases: false,
        output_mode: crate::process::OutputMode::Capture,
        ssh_user: None,
        allow_reboot: true,
    };

    // When activation fails, push() returns Err
    let result = {
        let push_ctx = PushContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_detector,
            &config,
            &observer,
        );
        push(&push_ctx, hosts)
    };

    // Push should succeed but with failures captured
    let results = result.expect("Push should succeed structurally");
    assert!(!results.all_succeeded(), "Push should have failures");
    assert_eq!(results.failure_count(), 1);

    // Verify the error message includes the source error chain
    let host_result = &results.phases[0].hosts[0];
    assert!(!host_result.success);
    let error_msg = host_result
        .error_message
        .as_ref()
        .expect("Failed host should have error message");

    assert!(
        error_msg.contains("fail") || host_result.hostname.contains("fail"),
        "Error should be associated with host 'fail'. Got hostname: {}, error: {}",
        host_result.hostname,
        error_msg
    );
    // The error message should indicate deployment/activation failure
    assert!(
        error_msg.contains("deploy")
            || error_msg.contains("activation")
            || error_msg.contains("Activation")
            || error_msg.contains("failed"),
        "Error message should indicate failure. Got: {}",
        error_msg
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
        ..Default::default()
    };
    let reboot_detector = crate::backends::reboot::SystemRebootBackend::new(reboot_config);
    let observer = crate::observer::NoopObserver;

    let hosts = vec![HostMetadata {
        name: "test".to_string(),
        fqdn: "test.example.com".to_string(),
        phase: Some("p1".to_string()),
        tags: vec![],
        reboot_policy: crate::nix::RebootPolicy::Auto,
        system_type: SystemType::NixOS,
    }];

    let config = PushConfiguration {
        strategy: PushStrategy::Switch,
        force: false,
        concurrency: 1,
        ignore_phases: false,
        output_mode: crate::process::OutputMode::Capture,
        ssh_user: None,
        allow_reboot: true,
    };

    // When activation fails, push() returns Err
    let result = {
        let push_ctx = PushContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_detector,
            &config,
            &observer,
        );
        push(&push_ctx, hosts)
    };

    // Push should succeed but with failures captured
    let results = result.expect("Push should succeed structurally");
    assert!(!results.all_succeeded(), "Push should have failures");
    assert_eq!(results.failure_count(), 1);

    // Verify the error message includes expected details
    let host_result = &results.phases[0].hosts[0];
    assert!(!host_result.success);
    let error_msg = host_result
        .error_message
        .as_ref()
        .expect("Failed host should have error message");

    assert!(
        error_msg.contains("test") || host_result.hostname.contains("test"),
        "Error should be associated with host 'test'. Got hostname: {}, error: {}",
        host_result.hostname,
        error_msg
    );
    // Error message should indicate deployment failure
    // (The exact message depends on mock backend implementation)
    assert!(
        error_msg.contains("deploy")
            || error_msg.contains("failed")
            || error_msg.contains("systemd"),
        "Error message should indicate deployment failure. Got: {}",
        error_msg
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
            reboot_policy: crate::nix::RebootPolicy::Never,
            system_type: SystemType::NixOS,
        },
        HostMetadata {
            name: "web2".to_string(),
            fqdn: "web2.example.com".to_string(),
            phase: Some("web".to_string()),
            tags: vec![],
            reboot_policy: crate::nix::RebootPolicy::Never,
            system_type: SystemType::NixOS,
        },
    ];

    let config = PushConfiguration {
        strategy: PushStrategy::Switch,
        force: false,
        concurrency: 2,
        ignore_phases: false,
        output_mode: crate::process::OutputMode::Capture,
        ssh_user: None,
        allow_reboot: true,
    };

    let results = {
        let push_ctx = PushContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_detector,
            &config,
            &observer,
        );
        push(&push_ctx, hosts)
    }
    .unwrap();

    // Verify both hosts succeeded
    assert_eq!(results.success_count(), 2);
    assert_eq!(results.failure_count(), 0);

    // Verify observer events fired in correct order (parallel-per-host model)
    let events = observer.get_events();

    // Build events (per-host)
    assert!(events.contains(&"build_start:web1".to_string()));
    assert!(events.contains(&"build_complete:web1:ok".to_string()));
    assert!(events.contains(&"build_start:web2".to_string()));
    assert!(events.contains(&"build_complete:web2:ok".to_string()));

    // Copy events (per-host)
    assert!(events.contains(&"copy_start:web1".to_string()));
    assert!(events.contains(&"copy_complete:web1:ok".to_string()));
    assert!(events.contains(&"copy_start:web2".to_string()));
    assert!(events.contains(&"copy_complete:web2:ok".to_string()));

    // Activation phase events
    assert!(events.contains(&"phase_start:web:2".to_string()));
    assert!(events.contains(&"activation_start:web1".to_string()));
    assert!(events.contains(&"activation_complete:web1:ok".to_string()));
    assert!(events.contains(&"activation_start:web2".to_string()));
    assert!(events.contains(&"activation_complete:web2:ok".to_string()));
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
            system_type: SystemType::NixOS,
        },
        HostMetadata {
            name: "web1".to_string(),
            fqdn: "web1.example.com".to_string(),
            phase: Some("web".to_string()),
            tags: vec![],
            reboot_policy: crate::nix::RebootPolicy::Never,
            system_type: SystemType::NixOS,
        },
    ];

    let config = PushConfiguration {
        strategy: PushStrategy::Switch,
        force: false,
        concurrency: 2,
        ignore_phases: false,
        output_mode: crate::process::OutputMode::Capture,
        ssh_user: None,
        allow_reboot: true,
    };

    let results = {
        let push_ctx = PushContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_detector,
            &config,
            &observer,
        );
        push(&push_ctx, hosts)
    }
    .unwrap();

    assert_eq!(results.success_count(), 2);

    let events = observer.get_events();

    // Database phase should complete before web phase starts (parallel-per-host model)
    let db_phase_start = events
        .iter()
        .position(|e| e == "phase_start:database:1")
        .expect("database phase should start");
    let db_phase_complete = events
        .iter()
        .position(|e| e == "phase_complete:database:ok")
        .expect("database phase should complete");
    let web_phase_start = events
        .iter()
        .position(|e| e == "phase_start:web:1")
        .expect("web phase should start");

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
        system_type: SystemType::NixOS,
    }];

    let config = PushConfiguration {
        strategy: PushStrategy::Switch,
        force: false,
        concurrency: 1,
        ignore_phases: false,
        output_mode: crate::process::OutputMode::Capture,
        ssh_user: None,
        allow_reboot: true,
    };

    let result = {
        let push_ctx = PushContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_detector,
            &config,
            &observer,
        );
        push(&push_ctx, hosts)
    };

    // Should succeed but with failures captured
    let results = result.unwrap();
    assert!(!results.all_succeeded());
    assert_eq!(results.failure_count(), 1);

    let events = observer.get_events();

    // Phase lifecycle events should be present
    assert!(events.contains(&"phase_start:default:1".to_string()));
    assert!(events.contains(&"phase_complete:default:failed".to_string()));

    // Build events should be present (parallel-per-host model)
    assert!(events.contains(&"build_start:fail-host".to_string()));
    assert!(events.contains(&"build_complete:fail-host:fail".to_string()));

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
        system_type: SystemType::NixOS,
    }];

    let config = PushConfiguration {
        strategy: PushStrategy::Switch,
        force: false,
        concurrency: 1,
        ignore_phases: false,
        output_mode: crate::process::OutputMode::Capture,
        ssh_user: None,
        allow_reboot: true,
    };

    let result = {
        let push_ctx = PushContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_detector,
            &config,
            &observer,
        );
        push(&push_ctx, hosts)
    };

    // Should succeed but with failures captured
    let results = result.unwrap();
    assert!(!results.all_succeeded());
    assert_eq!(results.failure_count(), 1);

    let events = observer.get_events();

    // Phase lifecycle events should be present
    assert!(events.contains(&"phase_start:default:1".to_string()));
    assert!(events.contains(&"phase_complete:default:failed".to_string()));

    // Build should succeed (parallel-per-host model)
    assert!(events.contains(&"build_start:fail-copy".to_string()));
    assert!(events.contains(&"build_complete:fail-copy:ok".to_string()));

    // Copy should fail
    assert!(events.contains(&"copy_start:fail-copy".to_string()));
    assert!(events.contains(&"copy_complete:fail-copy:fail".to_string()));

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
            system_type: SystemType::NixOS,
        },
        HostMetadata {
            name: "host2".to_string(),
            fqdn: "host2.example.com".to_string(),
            phase: Some("p1".to_string()),
            tags: vec![],
            reboot_policy: crate::nix::RebootPolicy::Never,
            system_type: SystemType::NixOS,
        },
    ];

    let config = PushConfiguration {
        strategy: PushStrategy::Switch,
        force: false,
        concurrency: 2,
        ignore_phases: false,
        output_mode: crate::process::OutputMode::Capture,
        ssh_user: None,
        allow_reboot: true,
    };

    {
        let push_ctx = PushContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_detector,
            &config,
            &observer,
        );
        push(&push_ctx, hosts)
    }
    .unwrap();

    let events = observer.get_events();

    // In parallel-per-host model, verify each host completes its copy
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

    // Each host's copy_start should come before its copy_complete
    assert!(copy1_start_idx < copy1_complete_idx);
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
            system_type: SystemType::NixOS,
        },
        HostMetadata {
            name: "host2".to_string(),
            fqdn: "host2.example.com".to_string(),
            phase: Some("p1".to_string()),
            tags: vec![],
            reboot_policy: crate::nix::RebootPolicy::Never,
            system_type: SystemType::NixOS,
        },
    ];

    let config = PushConfiguration {
        strategy: PushStrategy::Switch,
        force: false,
        concurrency: 2,
        ignore_phases: false,
        output_mode: crate::process::OutputMode::Capture,
        ssh_user: None,
        allow_reboot: true,
    };

    {
        let push_ctx = PushContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_detector,
            &config,
            &observer,
        );
        push(&push_ctx, hosts)
    }
    .unwrap();

    let events = observer.get_events();

    // In parallel-per-host model, verify each host completes its build
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

    // Each host's build_start should come before its build_complete
    assert!(build1_start_idx < build1_complete_idx);
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
            system_type: SystemType::NixOS,
        },
        HostMetadata {
            name: "host2".to_string(),
            fqdn: "host2.example.com".to_string(),
            phase: Some("p1".to_string()),
            tags: vec![],
            reboot_policy: crate::nix::RebootPolicy::Never,
            system_type: SystemType::NixOS,
        },
    ];

    let config = PushConfiguration {
        strategy: PushStrategy::Switch,
        force: false,
        concurrency: 2,
        ignore_phases: false,
        output_mode: crate::process::OutputMode::Capture,
        ssh_user: None,
        allow_reboot: true,
    };

    let result = {
        let push_ctx = PushContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_detector,
            &config,
            &observer,
        );
        push(&push_ctx, hosts)
    };

    // Should succeed but with failures captured
    let results = result.unwrap();
    assert!(!results.all_succeeded());
    assert_eq!(results.failure_count(), 1);
    assert_eq!(results.success_count(), 1);

    let events = observer.get_events();

    // Parallel-per-host model: host1 can proceed even if host2 fails
    // host1 build should succeed
    assert!(events.contains(&"build_complete:host1:ok".to_string()));

    // host2 build should fail
    assert!(events.contains(&"build_complete:host2:fail".to_string()));

    // In parallel model, host1 continues through the pipeline even though host2 failed
    // host1 should complete copy, activate, etc.
    assert!(events
        .iter()
        .any(|e| e.starts_with("copy_") && e.contains("host1")));

    // host2 should NOT proceed to copy (it failed build)
    assert!(!events
        .iter()
        .any(|e| e.starts_with("copy_") && e.contains("host2")));
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
            system_type: SystemType::NixOS,
        },
        HostMetadata {
            name: "web1".to_string(),
            fqdn: "web1.example.com".to_string(),
            phase: Some("web".to_string()),
            tags: vec![],
            reboot_policy: crate::nix::RebootPolicy::Never,
            system_type: SystemType::NixOS,
        },
    ];

    let config = PushConfiguration {
        strategy: PushStrategy::Switch,
        force: false,
        concurrency: 2,
        ignore_phases: true, // Ignore phase ordering
        output_mode: crate::process::OutputMode::Capture,
        ssh_user: None,
        allow_reboot: true,
    };

    let results = {
        let push_ctx = PushContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_detector,
            &config,
            &observer,
        );
        push(&push_ctx, hosts)
    }
    .unwrap();

    assert_eq!(results.success_count(), 2);

    let events = observer.get_events();

    // Both hosts should be in "_all" phase (parallel-per-host model)
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
            system_type: SystemType::NixOS,
        },
        HostMetadata {
            name: "phase2-host".to_string(),
            fqdn: "phase2-host.example.com".to_string(),
            phase: Some("phase2".to_string()),
            tags: vec![],
            reboot_policy: crate::nix::RebootPolicy::Never,
            system_type: SystemType::NixOS,
        },
    ];

    let config = PushConfiguration {
        strategy: PushStrategy::Switch,
        force: false,
        concurrency: 1,
        ignore_phases: false,
        output_mode: crate::process::OutputMode::Capture,
        ssh_user: None,
        allow_reboot: true,
    };

    // When activation fails, push() returns Err but observer events still fire
    let result = {
        let push_ctx = PushContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_detector,
            &config,
            &observer,
        );
        push(&push_ctx, hosts)
    };

    // Push should succeed but with failures and skipped phases
    let results = result.expect("Push should succeed structurally");
    assert!(!results.all_succeeded(), "Push should have failures");
    assert_eq!(results.failure_count(), 1);
    assert_eq!(results.skipped_phases.len(), 1, "Phase2 should be skipped");

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
        RebootReason::KernelChanged {
            old: "/nix/store/old-kernel".to_string(),
            new: "/nix/store/new-kernel".to_string(),
        },
    );
    let reboot_backend = MockRebootBackend::new().with_reboot_required(reboot_required.clone());

    let observer = TestObserver::new();

    let hosts = vec![make_host_metadata(
        "reboot-host",
        Some("default"),
        crate::nix::RebootPolicy::Auto,
    )];

    let config = PushConfiguration {
        strategy: PushStrategy::Auto, // Auto mode will detect reboot requirement
        force: false,
        concurrency: 1,
        ignore_phases: false,
        output_mode: crate::process::OutputMode::Capture,
        ssh_user: None,
        allow_reboot: true,
    };

    let results = {
        let push_ctx = PushContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_backend,
            &config,
            &observer,
        );
        push(&push_ctx, hosts)
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
    let activation_complete_idx = events
        .iter()
        .position(|e| e.starts_with("activation_complete:reboot-host"))
        .unwrap();

    assert!(
        activation_complete_idx < reboot_start_idx,
        "Reboot should happen after activation"
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
        RebootReason::InitrdChanged {
            old: "/nix/store/old-initrd".to_string(),
            new: "/nix/store/new-initrd".to_string(),
        },
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
        strategy: PushStrategy::Auto,
        force: false,
        concurrency: 1,
        ignore_phases: false,
        output_mode: crate::process::OutputMode::Capture,
        ssh_user: None,
        allow_reboot: true,
    };

    let results = {
        let push_ctx = PushContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_backend,
            &config,
            &observer,
        );
        push(&push_ctx, hosts)
    }
    .unwrap();

    // Should report failure due to reboot failure
    assert!(!results.all_succeeded());
    assert_eq!(results.failure_count(), 1);

    // Check that deployment itself succeeded but reboot failed
    let host_result = &results.phases[0].hosts[0];
    assert_eq!(host_result.hostname, "fail-reboot");
    assert!(!host_result.success);
    assert!(
        host_result
            .error_message
            .as_ref()
            .unwrap()
            .contains("reboot failed"),
        "Error should mention reboot failure: {:?}",
        host_result.error_message
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
fn test_push_skip_reboot_when_not_required() {
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
        strategy: PushStrategy::Auto,
        force: false,
        concurrency: 1,
        ignore_phases: false,
        output_mode: crate::process::OutputMode::Capture,
        ssh_user: None,
        allow_reboot: true,
    };

    let results = {
        let push_ctx = PushContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_backend,
            &config,
            &observer,
        );
        push(&push_ctx, hosts)
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

    // Should have completed activation
    assert!(events.iter().any(|e| e.starts_with("activation_complete")));
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
        RebootReason::KernelChanged {
            old: "/nix/store/old-kernel".to_string(),
            new: "/nix/store/new-kernel".to_string(),
        },
    );
    let reboot_backend = MockRebootBackend::new().with_reboot_required(reboot_required);

    let observer = TestObserver::new();

    let hosts = vec![make_host_metadata(
        "boot-host",
        Some("default"),
        crate::nix::RebootPolicy::Auto,
    )];

    let config = PushConfiguration {
        strategy: PushStrategy::Boot, // Explicit Boot mode
        force: false,
        concurrency: 1,
        ignore_phases: false,
        output_mode: crate::process::OutputMode::Capture,
        ssh_user: None,
        allow_reboot: true,
    };

    let results = {
        let push_ctx = PushContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_backend,
            &config,
            &observer,
        );
        push(&push_ctx, hosts)
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
        strategy: PushStrategy::Auto,
        force: false,
        concurrency: 1,
        ignore_phases: false,
        output_mode: crate::process::OutputMode::Capture,
        ssh_user: None,
        allow_reboot: true,
    };

    let results = {
        let push_ctx = PushContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_backend,
            &config,
            &observer,
        );
        push(&push_ctx, hosts)
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
        RebootReason::KernelChanged {
            old: "/nix/store/old-kernel".to_string(),
            new: "/nix/store/new-kernel".to_string(),
        },
    );
    let reboot_backend = MockRebootBackend::new().with_reboot_required(reboot_required);

    let observer = TestObserver::new();

    let hosts = vec![make_host_metadata(
        "never-reboot",
        Some("default"),
        crate::nix::RebootPolicy::Never, // Never reboot
    )];

    let config = PushConfiguration {
        strategy: PushStrategy::Auto,
        force: false,
        concurrency: 1,
        ignore_phases: false,
        output_mode: crate::process::OutputMode::Capture,
        ssh_user: None,
        allow_reboot: true,
    };

    let results = {
        let push_ctx = PushContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_backend,
            &config,
            &observer,
        );
        push(&push_ctx, hosts)
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

    // Should still have activated successfully
    assert!(events.iter().any(|e| e.starts_with("activation_complete")));
}

#[test]
fn test_push_skip_reboot_flag_overrides_always_policy() {
    // Test that allow_reboot=false prevents reboot even with RebootPolicy::Always
    let flake = Flake::test_context();
    let nix_backend = MockNixBackend::new();
    let deployer = MockDeployBackend::new();

    // Policy is Always, but user says --skip-reboot
    use crate::backends::mock::MockRebootBackend;
    let reboot_backend = MockRebootBackend::new();

    let observer = TestObserver::new();

    let hosts = vec![make_host_metadata(
        "always-reboot",
        Some("default"),
        crate::nix::RebootPolicy::Always, // Policy says always reboot
    )];

    let config = PushConfiguration {
        strategy: PushStrategy::Auto,
        force: false,
        concurrency: 1,
        ignore_phases: false,
        output_mode: crate::process::OutputMode::Capture,
        ssh_user: None,
        allow_reboot: false,
    };

    let results = {
        let push_ctx = PushContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_backend,
            &config,
            &observer,
        );
        push(&push_ctx, hosts)
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

    // Should still have activated successfully
    assert!(events.iter().any(|e| e.starts_with("activation_complete")));
}

#[test]
fn test_push_skip_reboot_flag_with_detected_reboot() {
    // Test that allow_reboot=false prevents reboot even when detection says it's needed
    let flake = Flake::test_context();
    let nix_backend = MockNixBackend::new();
    let deployer = MockDeployBackend::new();

    // Reboot detected as required but user says --skip-reboot
    use crate::backends::mock::MockRebootBackend;
    use crate::backends::reboot::RebootReason;
    let mut reboot_required = std::collections::HashMap::new();
    reboot_required.insert(
        "needs-reboot.example.com".to_string(),
        RebootReason::KernelChanged {
            old: "/nix/store/old-kernel".to_string(),
            new: "/nix/store/new-kernel".to_string(),
        },
    );
    let reboot_backend = MockRebootBackend::new().with_reboot_required(reboot_required);

    let observer = TestObserver::new();

    let hosts = vec![make_host_metadata(
        "needs-reboot",
        Some("default"),
        crate::nix::RebootPolicy::Auto, // Auto policy
    )];

    let config = PushConfiguration {
        strategy: PushStrategy::Auto,
        force: false,
        concurrency: 1,
        ignore_phases: false,
        output_mode: crate::process::OutputMode::Capture,
        ssh_user: None,
        allow_reboot: false,
    };

    let results = {
        let push_ctx = PushContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_backend,
            &config,
            &observer,
        );
        push(&push_ctx, hosts)
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

    // Should still have activated successfully (Boot mode without reboot)
    assert!(events.iter().any(|e| e.starts_with("activation_complete")));
}

// ============================================================================
// Integration Tests for Profile Update Feature
// ============================================================================

fn create_profile_test_backends() -> (
    Flake,
    MockNixBackend,
    MockDeployBackend,
    SystemRebootBackend,
    TestObserver,
) {
    let flake = Flake::test_context();
    let nix_backend = MockNixBackend::new();
    let deployer = MockDeployBackend::new();
    let reboot_config = crate::backends::reboot::RebootDetectionConfig {
        critical_services: vec![],
        check_kernel: false,
        check_initrd: false,
        ..Default::default()
    };
    let reboot_detector = SystemRebootBackend::new(reboot_config);
    let observer = TestObserver::new();
    (flake, nix_backend, deployer, reboot_detector, observer)
}

fn create_nixos_host_metadata(name: &str, fqdn: &str) -> HostMetadata {
    HostMetadata {
        name: name.to_string(),
        fqdn: fqdn.to_string(),
        phase: Some("default".to_string()),
        tags: vec![],
        reboot_policy: crate::nix::RebootPolicy::Auto,
        system_type: SystemType::NixOS,
    }
}

fn create_push_config(strategy: PushStrategy, concurrency: usize) -> PushConfiguration {
    PushConfiguration {
        strategy,
        force: false,
        concurrency,
        ignore_phases: false,
        output_mode: crate::process::OutputMode::Capture,
        ssh_user: None,
        allow_reboot: true,
    }
}

#[test]
fn test_push_profile_update_for_switch_mode() {
    let (flake, nix_backend, deployer, reboot_detector, observer) = create_profile_test_backends();

    let hosts = vec![create_nixos_host_metadata(
        "test-host",
        "test-host.example.com",
    )];
    let config = create_push_config(PushStrategy::Switch, 1);

    let results = {
        let push_ctx = PushContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_detector,
            &config,
            &observer,
        );
        push(&push_ctx, hosts)
    }
    .unwrap();

    // Deployment should succeed
    assert!(results.all_succeeded());
    assert_eq!(results.success_count(), 1);
    assert_eq!(results.failure_count(), 0);

    let host_result = &results.phases[0].hosts[0];
    assert!(host_result.success);
    assert_eq!(host_result.activation_mode, Some(ActivationMode::Switch));

    // Verify profile update was called for Switch mode
    let profile_updates = deployer.get_profile_updates();
    assert_eq!(
        profile_updates.len(),
        1,
        "Profile update should be called once"
    );
    assert_eq!(profile_updates[0].0, "test-host.example.com");
    assert_eq!(profile_updates[0].1, ActivationMode::Switch);
}

#[test]
fn test_push_profile_update_skipped_for_test_mode() {
    let (flake, nix_backend, deployer, reboot_detector, observer) = create_profile_test_backends();

    let hosts = vec![create_nixos_host_metadata(
        "test-host",
        "test-host.example.com",
    )];
    let config = create_push_config(PushStrategy::Test, 1);

    let results = {
        let push_ctx = PushContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_detector,
            &config,
            &observer,
        );
        push(&push_ctx, hosts)
    }
    .unwrap();

    // Deployment should succeed
    assert!(results.all_succeeded());
    assert_eq!(results.success_count(), 1);

    let host_result = &results.phases[0].hosts[0];
    assert!(host_result.success);
    assert_eq!(host_result.activation_mode, Some(ActivationMode::Test));

    // Verify profile update was NOT called for Test mode (ephemeral)
    let profile_updates = deployer.get_profile_updates();
    assert_eq!(
        profile_updates.len(),
        0,
        "Profile update should not be called for Test mode"
    );
}

#[test]
fn test_push_profile_update_multiple_hosts() {
    let (flake, nix_backend, deployer, reboot_detector, observer) = create_profile_test_backends();

    let hosts = vec![
        create_nixos_host_metadata("host1", "host1.example.com"),
        create_nixos_host_metadata("host2", "host2.example.com"),
        create_nixos_host_metadata("host3", "host3.example.com"),
    ];
    let config = create_push_config(PushStrategy::Switch, 3);

    let results = {
        let push_ctx = PushContext::new(
            &flake,
            &nix_backend,
            &deployer,
            &reboot_detector,
            &config,
            &observer,
        );
        push(&push_ctx, hosts)
    }
    .unwrap();

    // All hosts should succeed
    assert!(results.all_succeeded());
    assert_eq!(results.success_count(), 3);
    assert_eq!(results.failure_count(), 0);

    // All hosts should have used Switch mode (profile update)
    for host_result in &results.phases[0].hosts {
        assert!(host_result.success);
        assert_eq!(host_result.activation_mode, Some(ActivationMode::Switch));
    }

    // Verify profile update was called for all hosts
    let profile_updates = deployer.get_profile_updates();
    assert_eq!(
        profile_updates.len(),
        3,
        "Profile update should be called once per host"
    );

    // Verify each host had profile update called with Switch mode
    for host in [
        "host1.example.com",
        "host2.example.com",
        "host3.example.com",
    ] {
        assert!(
            deployer.profile_update_called_for(host),
            "Profile update should be called for {}",
            host
        );
    }
}
